use crate::{
    http::{access::AccessHttp, collect, curate, endpoints, knowledge, service_addresses, sites},
    runtime::curation::Curation,
    wiring::{Config, Databases},
};
use nexofolio_access::LoginService;
use nexofolio_access_adapter::{ConfiguredEmergencyPassword, PostgresAccess, Zentao};
use nexofolio_common::Result;
use nexofolio_curate::{Completions, Describe, Organize};
use nexofolio_curate_adapter::LlmClient;
use nexofolio_intake::{Intake, SystemClock};
use nexofolio_knowledge::Knowledge;
use nexofolio_observe::Observe;
use std::sync::Arc;

/// The business API: access, the site registry, collect and service
/// addresses. All of it needs
/// access configured, since collect targets and sites belong to its projects.
pub fn build_api(config: &Config, databases: &Databases) -> Result<Option<axum::Router>> {
    Ok(assemble_api(config, databases)?.map(|(router, _)| router))
}

/// Only the serving API starts recovery. Admin/preflight construction stays idle.
pub fn build_api_with_recovery(
    config: &Config,
    databases: &Databases,
    shutdown: tokio_util::sync::CancellationToken,
) -> Result<(Option<axum::Router>, Option<tokio::task::JoinHandle<()>>)> {
    let Some((router, curation)) = assemble_api(config, databases)? else {
        return Ok((None, None));
    };
    let recovery = tokio::spawn(curation.recover_after_restart(shutdown));
    Ok((Some(router), Some(recovery)))
}

fn assemble_api(
    config: &Config,
    databases: &Databases,
) -> Result<Option<(axum::Router, Curation)>> {
    let (Some(base), Some(key)) = (&config.zentao_base_url, &config.session_key) else {
        return Ok(None);
    };
    let provider = Arc::new(Zentao::new(base)?);
    let document_instance = provider.instance();
    let store = Arc::new(PostgresAccess::new(databases.access.clone(), key)?);
    let emergency = Arc::new(ConfiguredEmergencyPassword::new(
        config.emergency_password_hash.as_ref(),
    )?);
    let login = Arc::new(LoginService::new(
        provider.instance(),
        provider,
        emergency,
        store.clone(),
    ));
    let observe = Arc::new(Observe::new(databases.observe.clone()));
    let knowledge_svc = Arc::new(Knowledge::new(
        databases.knowledge.clone(),
        Observe::new(databases.observe.clone()),
    ));
    let intake = Intake::new(
        store.clone(),
        observe.clone(),
        Arc::new(databases.intake.clone()),
        Arc::new(SystemClock),
        config.collect_limits,
    )
    .with_project_lifecycle(store.clone());

    // Curate: LLM-powered describe and organize passes
    let completions: Arc<dyn Completions> = Arc::new(
        LlmClient::new(
            config
                .catalog_model_base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434/v1".into()),
            config
                .catalog_model
                .clone()
                .unwrap_or_else(|| "deepseek-v4-pro".into()),
            config.catalog_model_api_key.clone().unwrap_or_default(),
        )
        .with_concurrency(config.model_max_concurrency)
        .with_options(config.model_options),
    );
    let describe = Arc::new(Describe::new(
        completions.clone(),
        config
            .catalog_model
            .clone()
            .unwrap_or_else(|| "deepseek-v4-pro".into()),
    ));
    let organize = Arc::new(Organize::new(
        completions,
        config
            .catalog_model
            .clone()
            .unwrap_or_else(|| "deepseek-v4-pro".into()),
    ));
    let curation = Curation::new(
        knowledge_svc.clone(),
        knowledge_svc.clone(),
        observe.clone(),
        describe,
        organize,
    )
    .with_candidates(knowledge_svc.clone())
    .with_concurrency(
        config.field_concurrency.min(config.model_max_concurrency),
        config
            .description_concurrency
            .min(config.model_max_concurrency),
    )
    .with_idle_timeout(config.curation_idle_timeout);

    let administration = Arc::new(crate::runtime::project_management::ProjectAdministration {
        access: store.clone(),
        knowledge: databases.knowledge.clone(),
        observe: databases.observe.clone(),
        curation: curation.clone(),
    });
    let access = AccessHttp::new(login, store.clone(), store.clone(), store.clone());
    let router = crate::http::access::routes(access)
        .merge(crate::http::public_documents::routes(
            document_instance,
            Arc::new(databases.access.clone()),
            observe.clone(),
            knowledge_svc.clone(),
        ))
        .merge(crate::audit::console::routes(
            crate::audit::console::Services {
                sessions: store.clone(),
                projects: store.clone(),
                actors: store.clone(),
                store: Arc::new(databases.audit.clone()),
                candidates: knowledge_svc.clone(),
                knowledge: knowledge_svc.clone(),
                endpoints: observe.clone(),
            },
        ))
        .merge(crate::audit::routes::routes(
            store.clone(),
            Arc::new(databases.audit.clone()),
            Some(store.clone()),
        ))
        .merge(crate::http::mcp_tokens::routes(
            store.clone(),
            Arc::new(databases.access.clone()),
        ))
        .merge(crate::http::project_management::routes(
            store.clone(),
            administration,
        ))
        .merge(crate::http::user_management::routes(
            store.clone(),
            store.clone(),
        ))
        .merge(sites::routes(store.clone(), store.clone(), store.clone()))
        .merge(crate::http::profile::routes(
            store.clone(),
            Arc::new(nexofolio_access::ProfileService::new(store.clone())),
        ))
        .merge(endpoints::routes(
            store.clone(),
            store.clone(),
            observe.clone(),
            knowledge_svc.clone(),
        ))
        .merge(crate::http::document_lifecycle::routes(
            store.clone(),
            store.clone(),
            knowledge_svc.clone(),
            knowledge_svc.clone(),
        ))
        .merge(knowledge::routes(
            store.clone(),
            store.clone(),
            knowledge_svc.clone(),
            observe.clone(),
        ))
        .merge(curate::routes(
            store.clone(),
            store.clone(),
            curation.clone(),
        ))
        .merge(crate::http::curation_review::routes(
            store.clone(),
            store.clone(),
            knowledge_svc,
            store.clone(),
        ))
        .merge(service_addresses::routes(
            store.clone(),
            store.clone(),
            observe,
        ))
        .merge(collect::routes_with_updates(
            Arc::new(intake),
            databases.intake.clone(),
        ))
        .layer(axum::middleware::from_fn_with_state(
            crate::http::project_writes::ProjectWriteGate {
                sessions: store.clone(),
                lifecycle: store,
            },
            crate::http::project_writes::protect,
        ));
    Ok(Some((router, curation)))
}

/// MCP composes saved knowledge and access ports, independently of business model configuration.
pub fn build_navigation(databases: &Databases) -> crate::mcp::Navigation {
    crate::mcp::Navigation::new(
        Arc::new(databases.access.clone()),
        Arc::new(Observe::new(databases.observe.clone())),
        Arc::new(Knowledge::new(
            databases.knowledge.clone(),
            Observe::new(databases.observe.clone()),
        )),
    )
}

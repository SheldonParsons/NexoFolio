use crate::{
    http::{access::AccessHttp, collect, endpoints, service_addresses, sites},
    wiring::{Config, Databases},
};
use nexofolio_access::LoginService;
use nexofolio_access_adapter::{ConfiguredEmergencyPassword, PostgresAccess, Zentao};
use nexofolio_common::Result;
use nexofolio_intake::{Intake, SystemClock};
use nexofolio_observe::Observe;
use std::sync::Arc;

/// The business API: access, the site registry, collect and service
/// addresses. All of it needs
/// access configured, since collect targets and sites belong to its projects.
pub fn build_api(config: &Config, databases: &Databases) -> Result<Option<axum::Router>> {
    let (Some(base), Some(key)) = (&config.zentao_base_url, &config.session_key) else {
        return Ok(None);
    };
    let provider = Arc::new(Zentao::new(base)?);
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
    let intake = Intake::new(
        store.clone(),
        observe.clone(),
        Arc::new(databases.intake.clone()),
        Arc::new(SystemClock),
        config.collect_limits,
    );
    let access = AccessHttp::new(login, store.clone(), store.clone(), store.clone());
    Ok(Some(
        crate::http::access::routes(access)
            .merge(sites::routes(store.clone(), store.clone(), store.clone()))
            .merge(endpoints::routes(
                store.clone(),
                store.clone(),
                observe.clone(),
            ))
            .merge(service_addresses::routes(store.clone(), store, observe))
            .merge(collect::routes(Arc::new(intake))),
    ))
}

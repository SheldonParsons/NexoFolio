use crate::{
    http::{access::AccessHttp, collect, sites},
    wiring::{Config, Databases},
};
use async_trait::async_trait;
use nexofolio_access::LoginService;
use nexofolio_access_adapter::{ConfiguredEmergencyPassword, PostgresAccess, Zentao};
use nexofolio_common::Result;
use nexofolio_contracts::observation::{CanonicalObservation, ObservationSink, SinkError};
use nexofolio_intake::{Intake, SystemClock};
use std::sync::Arc;

/// The business API: access, the site registry and collect. All of it needs
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
    let intake = Intake::new(
        store.clone(),
        Arc::new(UntilObserve),
        Arc::new(databases.intake.clone()),
        Arc::new(SystemClock),
        config.collect_limits,
    );
    let access = AccessHttp::new(login, store.clone(), store.clone(), store.clone());
    Ok(Some(
        crate::http::access::routes(access)
            .merge(sites::routes(store.clone(), store.clone(), store))
            .merge(collect::routes(Arc::new(intake))),
    ))
}

/// Stands in for observe until stage 2c: accepts every batch and only logs
/// how many observations it would have stored.
struct UntilObserve;

#[async_trait]
impl ObservationSink for UntilObserve {
    async fn accept(
        &self,
        observations: Vec<CanonicalObservation>,
    ) -> std::result::Result<(), SinkError> {
        tracing::info!(
            observations = observations.len(),
            "observations_discarded_until_observe"
        );
        Ok(())
    }
}

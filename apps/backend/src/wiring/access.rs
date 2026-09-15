use crate::{http::access::AccessHttp, wiring::Config};
use nexofolio_application::LoginService;
use nexofolio_contracts::Result;
use nexofolio_infrastructure::{ConfiguredEmergencyPassword, Postgres, PostgresAccess, Zentao};
use std::sync::Arc;

pub fn build_access(
    config: &Config,
    database: Postgres,
) -> Result<Option<crate::http::services::BackendServices>> {
    let (Some(base), Some(key)) = (&config.zentao_base_url, &config.session_key) else {
        return Ok(None);
    };
    let provider = Arc::new(Zentao::new(base)?);
    let store = Arc::new(PostgresAccess::new(database.clone(), key)?);
    let emergency = Arc::new(ConfiguredEmergencyPassword::new(
        config.emergency_password_hash.as_ref(),
    )?);
    let login = Arc::new(LoginService::new(
        provider.instance(),
        provider,
        emergency,
        store.clone(),
    ));
    let access = AccessHttp::new(login, store.clone());
    super::services::build_services(config, database, store, access).map(Some)
}

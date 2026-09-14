use crate::{http::access::AccessHttp, wiring::Config};
use nexofolio_application::LoginService;
use nexofolio_contracts::Result;
use nexofolio_infrastructure::{ConfiguredEmergencyPassword, Postgres, PostgresAccess, Zentao};
use std::sync::Arc;

pub fn build_access(config: &Config, database: Postgres) -> Result<Option<AccessHttp>> {
    let (Some(base), Some(key)) = (&config.zentao_base_url, &config.session_key) else {
        return Ok(None);
    };
    let provider = Arc::new(Zentao::new(base)?);
    let store = Arc::new(PostgresAccess::new(database, key)?);
    let emergency = Arc::new(ConfiguredEmergencyPassword::new(
        config.emergency_password_hash.as_ref(),
    )?);
    let login = Arc::new(LoginService::new(
        provider.instance(),
        provider,
        emergency,
        store.clone(),
    ));
    Ok(Some(AccessHttp::new(login, store)))
}

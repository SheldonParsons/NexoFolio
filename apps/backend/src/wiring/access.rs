use crate::{http::access::AccessHttp, wiring::Config};
use nexofolio_access::LoginService;
use nexofolio_access_adapter::{ConfiguredEmergencyPassword, Postgres, PostgresAccess, Zentao};
use nexofolio_common::Result;
use std::sync::Arc;

pub fn build_access(config: &Config, database: Postgres) -> Result<Option<axum::Router>> {
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
    Ok(Some(crate::http::access::routes(access)))
}

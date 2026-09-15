use nexofolio_contracts::{Error, Result, Secret};
use std::{net::SocketAddr, time::Duration};

#[derive(Debug)]
pub struct CatalogModelConfig {
    pub base_url: String,
    pub model: String,
    pub api_key_file: std::path::PathBuf,
}

#[derive(Debug)]
pub struct Config {
    pub maintenance_vision: bool,
    pub maintenance_context_tokens: usize,
    pub maintenance_snapshot_bytes: usize,
    pub maintenance_model_timeout: Duration,
    pub maintenance_max_calls: u32,
    pub capture_enabled: bool,
    pub blob_root: std::path::PathBuf,
    pub catalog_model: Option<CatalogModelConfig>,
    pub bind_addr: SocketAddr,
    pub database_url: Secret,
    pub database_max_connections: u32,
    pub database_timeout: Duration,
    pub request_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub mcp_allowed_hosts: Vec<String>,
    pub mcp_allowed_origins: Vec<String>,
    pub log_filter: String,
    pub zentao_base_url: Option<String>,
    pub session_key: Option<Secret>,
    pub emergency_password_hash: Option<Secret>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Injectable lookup avoids changing process-global environment in tests.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let invalid = |name: &str| Error::InvalidInput {
            message: format!("{name} is missing or invalid"),
        };
        let database_url = lookup("DATABASE_URL")
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| invalid("DATABASE_URL"))?;
        let bind_addr = lookup("NEXOFOLIO_BIND_ADDR")
            .unwrap_or_else(|| "127.0.0.1:8080".into())
            .parse()
            .map_err(|_| invalid("NEXOFOLIO_BIND_ADDR"))?;
        let number = |name: &str, default: u64, max: u64| -> Result<u64> {
            let value = lookup(name)
                .map(|s| s.parse::<u64>())
                .transpose()
                .map_err(|_| invalid(name))?
                .unwrap_or(default);
            if value == 0 || value > max {
                return Err(invalid(name));
            }
            Ok(value)
        };
        let list = |name: &str, default: &str| -> Result<Vec<String>> {
            let value = lookup(name).unwrap_or_else(|| default.into());
            let items: Vec<_> = value.split(',').map(str::trim).map(str::to_owned).collect();
            if items.iter().any(|s| s.is_empty() || s == "*") {
                return Err(invalid(name));
            }
            Ok(items)
        };
        let model_keys = [
            "NEXOFOLIO_CATALOG_MODEL_BASE_URL",
            "NEXOFOLIO_CATALOG_MODEL",
            "NEXOFOLIO_CATALOG_MODEL_API_KEY_FILE",
        ];
        let model_values: Vec<_> = model_keys
            .iter()
            .map(|k| lookup(k).filter(|s| !s.trim().is_empty()))
            .collect();
        let catalog_model = if model_values.iter().all(Option::is_none) {
            None
        } else {
            if model_values.iter().any(Option::is_none) {
                return Err(invalid(
                    "catalog model base URL, model, and API key file (configure all three)",
                ));
            }
            Some(CatalogModelConfig {
                base_url: model_values[0].clone().unwrap(),
                model: model_values[1].clone().unwrap(),
                api_key_file: model_values[2].clone().unwrap().into(),
            })
        };
        let config = Self {
            maintenance_vision: lookup("NEXOFOLIO_MAINTENANCE_VISION")
                .is_some_and(|v| v == "true" || v == "1"),
            maintenance_context_tokens: number(
                "NEXOFOLIO_MAINTENANCE_CONTEXT_TOKENS",
                65536,
                1048576,
            )? as usize,
            maintenance_model_timeout: Duration::from_millis(number(
                "NEXOFOLIO_MAINTENANCE_MODEL_TIMEOUT_MS",
                240000,
                240000,
            )?),
            maintenance_snapshot_bytes: number(
                "NEXOFOLIO_MAINTENANCE_SNAPSHOT_BYTES",
                64 * 1024 * 1024,
                1024 * 1024 * 1024,
            )? as usize,
            maintenance_max_calls: number("NEXOFOLIO_MAINTENANCE_MAX_CALLS", 256, 4096)? as u32,
            capture_enabled: lookup("NEXOFOLIO_CAPTURE_ENABLED")
                .is_some_and(|v| v == "true" || v == "1"),
            blob_root: lookup("NEXOFOLIO_BLOB_ROOT")
                .unwrap_or_else(|| "./data/blobs".into())
                .into(),
            catalog_model,
            bind_addr,
            database_url: Secret::new(database_url),
            database_max_connections: number("NEXOFOLIO_DB_MAX_CONNECTIONS", 5, 100)? as u32,
            database_timeout: Duration::from_millis(number(
                "NEXOFOLIO_DB_TIMEOUT_MS",
                1500,
                30000,
            )?),
            request_timeout: Duration::from_millis(number(
                "NEXOFOLIO_REQUEST_TIMEOUT_MS",
                60000,
                120000,
            )?),
            shutdown_timeout: Duration::from_millis(number(
                "NEXOFOLIO_SHUTDOWN_TIMEOUT_MS",
                5000,
                30000,
            )?),
            mcp_allowed_hosts: list("NEXOFOLIO_MCP_ALLOWED_HOSTS", "localhost,127.0.0.1,[::1]")?,
            mcp_allowed_origins: list(
                "NEXOFOLIO_MCP_ALLOWED_ORIGINS",
                "http://localhost,http://127.0.0.1",
            )?,
            zentao_base_url: lookup("NEXOFOLIO_ZENTAO_BASE_URL").filter(|s| !s.trim().is_empty()),
            session_key: lookup("NEXOFOLIO_SESSION_KEY")
                .filter(|s| !s.is_empty())
                .map(Secret::new),
            emergency_password_hash: lookup("NEXOFOLIO_EMERGENCY_PASSWORD_HASH")
                .filter(|s| !s.is_empty())
                .map(Secret::new),
            log_filter: lookup("NEXOFOLIO_LOG").unwrap_or_else(|| "info".into()),
        };
        if config.zentao_base_url.is_some() != config.session_key.is_some() {
            return Err(invalid(
                "NEXOFOLIO_ZENTAO_BASE_URL and NEXOFOLIO_SESSION_KEY (configure both)",
            ));
        }
        if config.emergency_password_hash.is_some() && config.session_key.is_none() {
            return Err(invalid(
                "NEXOFOLIO_SESSION_KEY (required for emergency login)",
            ));
        }
        if config.database_timeout >= config.request_timeout {
            return Err(invalid(
                "NEXOFOLIO_DB_TIMEOUT_MS (must be below request timeout)",
            ));
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_and_invalid_configuration_is_actionable_without_values() {
        assert!(
            Config::from_lookup(|_| None)
                .unwrap_err()
                .to_string()
                .contains("DATABASE_URL")
        );
        let err = Config::from_lookup(|key| match key {
            "DATABASE_URL" => Some("postgres://secret:password@localhost/db".into()),
            "NEXOFOLIO_DB_MAX_CONNECTIONS" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("NEXOFOLIO_DB_MAX_CONNECTIONS"));
        assert!(!err.to_string().contains("password"));
    }
    #[test]
    fn debug_configuration_redacts_database_url() {
        let c = Config::from_lookup(|key| {
            (key == "DATABASE_URL").then(|| "postgres://secret:password@localhost/db".into())
        })
        .unwrap();
        assert!(!format!("{c:?}").contains("secret:password"));
        assert_eq!(c.bind_addr.port(), 8080);
    }
}

use nexofolio_access_adapter::Unconfigured;
use nexofolio_access_contracts::McpTokenVerifier;
use nexofolio_common::{Error, Secret};

#[tokio::test]
async fn absent_mcp_tokens_never_authenticate() {
    assert!(matches!(
        Unconfigured.verify(&Secret::new("any-token")).await,
        Err(Error::NotConfigured { .. })
    ));
}

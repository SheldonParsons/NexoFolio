use nexofolio_access_adapter::Unconfigured;
use nexofolio_access_contracts::{
    EmergencyAuthenticator, ExternalAuthenticator, LoginCredentials, McpTokenVerifier,
    ProjectPermissionSource, ProjectSource, UserDirectory,
};
use nexofolio_common::{Error, Secret, UserId};

fn not_configured<T>(result: nexofolio_common::Result<T>) {
    assert!(matches!(result, Err(Error::NotConfigured { .. })));
}

#[tokio::test]
async fn absent_integrations_never_invent_identity_permissions_or_success() {
    let provider = Unconfigured;
    let credentials = LoginCredentials {
        account: "existing-user".into(),
        password: Secret::new("test-only"),
    };
    not_configured(provider.authenticate(&credentials).await);
    not_configured(provider.authenticate_existing(&credentials).await);
    not_configured(provider.find_existing("existing-user").await);
    not_configured(provider.list_projects(None).await);
    not_configured(provider.grants_for(UserId::new()).await);
    not_configured(provider.verify(&Secret::new("any-token")).await);
}

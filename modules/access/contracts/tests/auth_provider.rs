use async_trait::async_trait;
use nexofolio_access_contracts::{ExternalAuthenticator, ExternalIdentity, LoginCredentials};
use nexofolio_common::{Error, Result, Secret};

struct FixtureProvider;
#[async_trait]
impl ExternalAuthenticator for FixtureProvider {
    async fn authenticate(&self, credentials: &LoginCredentials) -> Result<ExternalIdentity> {
        if credentials.account != "fixture" {
            return Err(Error::Unauthenticated);
        }
        Ok(ExternalIdentity {
            instance: "test-instance".into(),
            external_id: "external-user".into(),
            account: credentials.account.clone(),
            display_name: "Test user".into(),
        })
    }
}

#[tokio::test]
async fn external_identity_contract_is_testable_without_zentao_http_or_sql() {
    let provider: Box<dyn ExternalAuthenticator> = Box::new(FixtureProvider);
    let credentials = LoginCredentials {
        account: "fixture".into(),
        password: Secret::new("test-only"),
    };
    assert_eq!(
        provider
            .authenticate(&credentials)
            .await
            .unwrap()
            .external_id,
        "external-user"
    );
    let invalid = LoginCredentials {
        account: "unknown".into(),
        password: Secret::new("test-only"),
    };
    assert!(matches!(
        provider.authenticate(&invalid).await,
        Err(Error::Unauthenticated)
    ));
}

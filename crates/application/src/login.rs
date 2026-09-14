use nexofolio_access::{
    EmergencyPassword, LoginCredentials, LoginProvider, PlatformAccess, SessionLogin,
    SessionPrincipal, SyncCounts,
};
use nexofolio_contracts::Result;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginSync {
    Completed { counts: SyncCounts },
    Failed { code: &'static str },
    Skipped { reason: &'static str },
}
pub struct LoginOutcome {
    pub session: SessionLogin,
    pub sync: LoginSync,
}

pub struct LoginService {
    instance: String,
    provider: Arc<dyn LoginProvider>,
    emergency: Arc<dyn EmergencyPassword>,
    store: Arc<dyn PlatformAccess>,
}
impl LoginService {
    pub fn new(
        instance: String,
        provider: Arc<dyn LoginProvider>,
        emergency: Arc<dyn EmergencyPassword>,
        store: Arc<dyn PlatformAccess>,
    ) -> Self {
        Self {
            instance,
            provider,
            emergency,
            store,
        }
    }
    pub async fn login(&self, credentials: LoginCredentials) -> Result<LoginOutcome> {
        if self.emergency.matches(&credentials.password).await? {
            let session = self
                .store
                .emergency_login(&self.instance, &credentials.account)
                .await?;
            return Ok(LoginOutcome {
                session,
                sync: LoginSync::Skipped {
                    reason: "EMERGENCY_LOGIN_NO_ZENTAO_CREDENTIAL",
                },
            });
        }
        // Even when a local session exists, ordinary login must verify ZenTao first.
        // Order the entire observation, including /user, before network requests.
        let generation = self.store.next_sync_generation().await?;
        let remote = self.provider.login(&credentials).await?;
        let session = self.store.normal_login(&remote.identity).await?;
        let sync = match self
            .sync(
                &SessionPrincipal {
                    user_id: session.user.id,
                    instance: remote.identity.instance.clone(),
                },
                &remote,
                generation,
            )
            .await
        {
            Ok(counts) => LoginSync::Completed { counts },
            Err(_) => LoginSync::Failed {
                code: "PROJECT_SYNC_FAILED",
            },
        };
        Ok(LoginOutcome { session, sync })
    }
    async fn sync(
        &self,
        principal: &SessionPrincipal,
        remote: &nexofolio_access::RemoteLogin,
        generation: i64,
    ) -> Result<SyncCounts> {
        let snapshot = self.provider.projects(remote).await?;
        self.store
            .apply_snapshot(principal, generation, snapshot)
            .await
    }
}

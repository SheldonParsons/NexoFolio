//! Transport identity comes from the authenticated session, never request JSON.
use nexofolio_access_contracts::{SessionPrincipal, Sessions};
use nexofolio_common::Result;
use nexofolio_contracts::knowledge::CandidateActor;
pub(super) async fn from_session(
    sessions: &dyn Sessions,
    principal: &SessionPrincipal,
) -> Result<CandidateActor> {
    let profile = sessions.me(principal).await?;
    if profile.id != principal.user_id {
        return Err(nexofolio_common::Error::Unauthenticated);
    }
    Ok(CandidateActor::User {
        id: principal.user_id,
        display_name: Some(profile.display_name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nexofolio_access_contracts::UserProfile;
    use nexofolio_common::{Secret, UserId};
    struct Session(UserId);
    #[async_trait]
    impl Sessions for Session {
        async fn verify_session(&self, _: &Secret) -> Result<SessionPrincipal> {
            Ok(SessionPrincipal {
                user_id: self.0,
                instance: "fixture".into(),
            })
        }
        async fn me(&self, _: &SessionPrincipal) -> Result<UserProfile> {
            Ok(UserProfile {
                id: self.0,
                account: "operator".into(),
                display_name: "已登录用户".into(),
                enabled: true,
                avatar_url: None,
            })
        }
    }
    #[tokio::test]
    async fn actor_is_the_verified_session_user_and_never_a_system_identity() {
        let id = UserId::new();
        let principal = SessionPrincipal {
            user_id: id,
            instance: "fixture".into(),
        };
        let actor = from_session(&Session(id), &principal).await.unwrap();
        assert!(
            matches!(actor, CandidateActor::User { id: actual, display_name: Some(name) } if actual == id && name == "已登录用户")
        );
        assert!(
            from_session(&Session(UserId::new()), &principal)
                .await
                .is_err()
        );
    }
}

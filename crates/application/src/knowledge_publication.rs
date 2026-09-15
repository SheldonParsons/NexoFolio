//! Publication policy is chosen by the application, never by a database adapter.
use crate::{KnowledgeActivationStore, KnowledgePublicationPolicy};
use nexofolio_contracts::*;
use std::sync::Arc;
use uuid::Uuid;

pub struct KnowledgePublicationService {
    store: Arc<dyn KnowledgeActivationStore>,
    policy: Arc<dyn KnowledgePublicationPolicy>,
}
impl KnowledgePublicationService {
    pub fn new(
        store: Arc<dyn KnowledgeActivationStore>,
        policy: Arc<dyn KnowledgePublicationPolicy>,
    ) -> Self {
        Self { store, policy }
    }
    pub async fn publish(
        &self,
        user: UserId,
        project: ProjectId,
        run: Uuid,
        request: &PublishKnowledge,
    ) -> Result<KnowledgeActivation> {
        self.policy.authorize_mode(true)?;
        self.store
            .publish_knowledge(user, project, run, request)
            .await
    }
    pub async fn restore(
        &self,
        user: UserId,
        project: ProjectId,
        request: &RestoreKnowledge,
    ) -> Result<KnowledgeActivation> {
        self.policy.authorize_mode(true)?;
        self.store.restore_knowledge(user, project, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KnowledgePublicationPolicy, ManualKnowledgePublication};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Deny;
    impl KnowledgePublicationPolicy for Deny {
        fn authorize_mode(&self, _: bool) -> Result<()> {
            Err(Error::Forbidden)
        }
    }
    #[derive(Default)]
    struct Store(AtomicUsize);
    #[async_trait::async_trait]
    impl KnowledgeActivationStore for Store {
        async fn publish_knowledge(
            &self,
            _: UserId,
            _: ProjectId,
            _: Uuid,
            _: &PublishKnowledge,
        ) -> Result<KnowledgeActivation> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::Conflict)
        }
        async fn restore_knowledge(
            &self,
            _: UserId,
            _: ProjectId,
            _: &RestoreKnowledge,
        ) -> Result<KnowledgeActivation> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::Conflict)
        }
        async fn knowledge_versions(
            &self,
            _: UserId,
            _: ProjectId,
            _: u32,
            _: u32,
        ) -> Result<KnowledgeVersionPage> {
            unreachable!()
        }
        async fn interface_knowledge(
            &self,
            _: UserId,
            _: ProjectId,
            _: InterfaceId,
            _: EnvironmentId,
        ) -> Result<InterfaceKnowledge> {
            unreachable!()
        }
    }
    #[tokio::test]
    async fn replacing_publication_policy_blocks_storage_without_changing_adapter() {
        let store = Arc::new(Store::default());
        let denied = KnowledgePublicationService::new(store.clone(), Arc::new(Deny));
        let publish = PublishKnowledge {
            request_id: Uuid::new_v4(),
            expected_generation: 0,
        };
        let restore = RestoreKnowledge {
            request_id: Uuid::new_v4(),
            expected_generation: 0,
            version_id: None,
        };
        assert!(matches!(
            denied
                .publish(UserId::new(), ProjectId::new(), Uuid::new_v4(), &publish)
                .await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            denied
                .restore(UserId::new(), ProjectId::new(), &restore)
                .await,
            Err(Error::Forbidden)
        ));
        assert_eq!(store.0.load(Ordering::SeqCst), 0);
        let allowed =
            KnowledgePublicationService::new(store.clone(), Arc::new(ManualKnowledgePublication));
        assert!(matches!(
            allowed
                .publish(UserId::new(), ProjectId::new(), Uuid::new_v4(), &publish)
                .await,
            Err(Error::Conflict)
        ));
        assert_eq!(store.0.load(Ordering::SeqCst), 1);
    }
}

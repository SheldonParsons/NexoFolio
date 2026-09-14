use async_trait::async_trait;
use nexofolio_access::*;
use nexofolio_application::*;
use nexofolio_contracts::*;
use nexofolio_intake::*;
use nexofolio_knowledge::*;
use nexofolio_rebuild::*;
use nexofolio_triggers::*;

/// Production default. Never authenticates, invents projects, returns fake records or runs jobs.
#[derive(Debug, Default)]
pub struct Unconfigured;

fn unavailable<T>(capability: &'static str) -> Result<T> {
    Err(Error::NotConfigured { capability })
}

#[async_trait]
impl ExternalAuthenticator for Unconfigured {
    async fn authenticate(&self, _: &LoginCredentials) -> Result<ExternalIdentity> {
        unavailable("zentao_auth")
    }
}
#[async_trait]
impl EmergencyAuthenticator for Unconfigured {
    async fn authenticate_existing(&self, _: &LoginCredentials) -> Result<UserProfile> {
        unavailable("emergency_auth")
    }
}
#[async_trait]
impl UserDirectory for Unconfigured {
    async fn find_existing(&self, _: &str) -> Result<Option<UserProfile>> {
        unavailable("users")
    }
}
#[async_trait]
impl ProjectSource for Unconfigured {
    async fn list_projects(&self, _: Option<&str>) -> Result<ProjectSourcePage> {
        unavailable("zentao_projects")
    }
}
#[async_trait]
impl ProjectPermissionSource for Unconfigured {
    async fn grants_for(&self, _: UserId) -> Result<Vec<ProjectGrant>> {
        unavailable("zentao_permissions")
    }
}
#[async_trait]
impl McpTokenVerifier for Unconfigured {
    async fn verify(&self, _: &Secret) -> Result<McpPrincipal> {
        unavailable("mcp_token_verification")
    }
}
#[async_trait]
impl McpTokenManager for Unconfigured {
    async fn issue(&self, _: IssueMcpToken) -> Result<IssuedMcpToken> {
        unavailable("mcp_token_issuance")
    }
    async fn revoke(&self, _: UserId, _: TokenId) -> Result<()> {
        unavailable("mcp_token_revocation")
    }
}
impl CaptureValidator for Unconfigured {
    fn validate(&self, _: &CaptureInput) -> Result<ValidationResult> {
        unavailable("capture_validation")
    }
}
impl IdentityResolver for Unconfigured {
    fn resolve(&self, _: &CaptureInput) -> Result<InterfaceIdentity> {
        unavailable("interface_identity")
    }
}
impl Fingerprinter for Unconfigured {
    fn fingerprint(&self, _: &CaptureInput) -> Result<StructuralFingerprint> {
        unavailable("fingerprint")
    }
}
#[async_trait]
impl Deduplicator for Unconfigured {
    async fn lookup(
        &self,
        _: &InterfaceIdentity,
        _: &StructuralFingerprint,
    ) -> Result<DeduplicationResult> {
        unavailable("deduplication")
    }
}
#[async_trait]
impl KnowledgeReader for Unconfigured {
    async fn read(
        &self,
        _: ProjectId,
        _: InterfaceId,
        _: Option<RevisionId>,
    ) -> Result<InterfaceRevision> {
        unavailable("knowledge_read")
    }
}
#[async_trait]
impl CatalogPublisher for Unconfigured {
    async fn publish(&self, _: PublicationCondition) -> Result<()> {
        unavailable("catalog_publication")
    }
}
#[async_trait]
impl TriggerPolicy for Unconfigured {
    async fn evaluate(&self, _: &KnowledgeEvent, _: &DirectoryMetrics) -> Result<TriggerDecision> {
        unavailable("trigger_policy")
    }
}
#[async_trait]
impl CatalogBuilder for Unconfigured {
    async fn build(&self, _: RebuildRequest) -> Result<CandidatePlan> {
        unavailable("catalog_builder")
    }
}
#[async_trait]
impl CatalogEvaluator for Unconfigured {
    async fn evaluate(&self, _: &CandidatePlan) -> Result<CandidateEvaluation> {
        unavailable("catalog_evaluator")
    }
}
#[async_trait]
impl JobDispatcher for Unconfigured {
    async fn dispatch(&self, _: JobRequest) -> Result<()> {
        unavailable("job_dispatch")
    }
}
#[async_trait]
impl CheckpointStore for Unconfigured {
    async fn save(&self, _: Checkpoint) -> Result<()> {
        unavailable("checkpoints")
    }
}
#[async_trait]
impl JobExecutor for Unconfigured {
    async fn execute(&self, _: JobRequest, _: ExecutionOwnership) -> Result<()> {
        unavailable("job_execution")
    }
}
#[async_trait]
impl KnowledgeTransaction for Unconfigured {
    async fn commit_revision(&self, _: CommitRevision) -> Result<ChangeId> {
        unavailable("knowledge_transaction")
    }
}

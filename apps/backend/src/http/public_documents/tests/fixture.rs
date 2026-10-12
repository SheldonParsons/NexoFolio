use super::super::*;
use async_trait::async_trait;
use nexofolio_access_contracts::*;
use nexofolio_common::{DatabaseProbe, Secret, TokenId, UserId};
use nexofolio_contracts::{
    endpoint::*,
    knowledge::*,
    observation::*,
    testing::{InMemoryAddresses, InMemoryEndpoints, sample_exchange},
};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Access {
    pub project: ProjectId,
    pub env: EnvironmentId,
    pub member: UserId,
    pub outsider: UserId,
}
#[async_trait]
impl ProjectDocuments for Access {
    async fn document_project(
        &self,
        instance: &str,
        project: ProjectId,
    ) -> nexofolio_common::Result<Option<DocumentProject>> {
        Ok(
            (instance == "fixture" && project == self.project).then(|| DocumentProject {
                project_id: project,
                name: "公开项目".into(),
                status: "doing".into(),
                environments: vec![Environment {
                    id: self.env,
                    name: "测试环境".into(),
                }],
            }),
        )
    }
}
#[async_trait]
impl Sessions for Access {
    async fn verify_session(&self, token: &Secret) -> nexofolio_common::Result<SessionPrincipal> {
        let user_id = match token.expose() {
            "member" => self.member,
            "outsider" => self.outsider,
            _ => return Err(Error::Unauthenticated),
        };
        Ok(SessionPrincipal {
            user_id,
            instance: "fixture".into(),
        })
    }
    async fn me(&self, p: &SessionPrincipal) -> nexofolio_common::Result<UserProfile> {
        Ok(UserProfile {
            id: p.user_id,
            account: "fixture".into(),
            display_name: "fixture".into(),
            enabled: true,
            avatar_url: None,
        })
    }
}
#[async_trait]
impl ProjectAccess for Access {
    async fn list_projects(
        &self,
        _: &SessionPrincipal,
        _: u32,
        _: u32,
    ) -> nexofolio_common::Result<ProjectPage> {
        Err(Error::Forbidden)
    }
    async fn require_project(
        &self,
        p: &SessionPrincipal,
        id: ProjectId,
    ) -> nexofolio_common::Result<ProjectCard> {
        if p.user_id != self.member || id != self.project {
            return Err(Error::Forbidden);
        }
        Ok(ProjectCard {
            project_id: id,
            name: "公开项目".into(),
            status: "doing".into(),
            origin: nexofolio_access_contracts::ProjectOrigin::Zentao,
            can_access: true,
            access_state: AccessState::Allowed,
            reason_code: None,
        })
    }
}
#[async_trait]
impl McpTokenVerifier for Access {
    async fn verify(&self, s: &Secret) -> nexofolio_common::Result<McpPrincipal> {
        let p = self.verify_session(s).await?;
        Ok(McpPrincipal {
            user_id: p.user_id,
            token_id: TokenId::new(),
        })
    }
}
#[async_trait]
impl McpProjectAccess for Access {
    async fn require_mcp_project(
        &self,
        p: &McpPrincipal,
        id: ProjectId,
    ) -> nexofolio_common::Result<()> {
        self.require_project(
            &SessionPrincipal {
                user_id: p.user_id,
                instance: "fixture".into(),
            },
            id,
        )
        .await
        .map(|_| ())
    }
    async fn list_mcp_projects(
        &self,
        _: &McpPrincipal,
        _: u64,
        _: u32,
    ) -> nexofolio_common::Result<McpDiscoveryPage<McpProject>> {
        Err(Error::Forbidden)
    }
    async fn list_mcp_environments(
        &self,
        _: &McpPrincipal,
        _: ProjectId,
        _: u64,
        _: u32,
    ) -> nexofolio_common::Result<McpDiscoveryPage<McpEnvironment>> {
        Err(Error::Forbidden)
    }
}
#[async_trait]
impl DatabaseProbe for Access {
    async fn check(&self) -> nexofolio_common::Result<()> {
        Ok(())
    }
}
#[derive(Clone)]
pub struct Reader {
    pub data: Arc<InMemoryEndpoints>,
    pub raw_reads: Arc<AtomicUsize>,
}
#[async_trait]
impl EndpointReader for Reader {
    async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        self.data.list(p).await
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        self.data.get(id).await
    }
    async fn example(&self, id: EndpointId, s: &str) -> Result<Option<Value>, EndpointError> {
        self.raw_reads.fetch_add(1, Ordering::SeqCst);
        self.data.example(id, s).await
    }
}
pub struct Fixture {
    pub app: Router,
    pub access: Arc<Access>,
    pub target: EndpointId,
    pub alias: EndpointId,
    pub foreign: EndpointId,
    pub reader: Reader,
    pub addresses: Arc<InMemoryAddresses>,
}
pub async fn setup() -> Fixture {
    let access = Arc::new(Access {
        project: ProjectId::new(),
        env: EnvironmentId::new(),
        member: UserId::new(),
        outsider: UserId::new(),
    });
    let target = EndpointId::new();
    let alias = EndpointId::new();
    let foreign = EndpointId::new();
    let reader = Reader {
        data: Arc::new(InMemoryEndpoints::new()),
        raw_reads: Arc::new(AtomicUsize::new(0)),
    };
    let mut raw = sample_exchange(access.project, access.env);
    if let Fact::Exchange(x) = &mut raw.fact {
        x.request.headers = Some(Headers {
            complete: true,
            entries: vec![
                ("Authorization".into(), "Bearer PRIVATE_AUTH".into()),
                ("Cookie".into(), "PRIVATE_COOKIE".into()),
            ],
        });
        x.request.body = Body::Full {
            media_type: Some("application/json".into()),
            content: Content::Text("{\"data\":\"PRIVATE_BODY\"}".into()),
        };
    }
    raw.context = Some(json!({"page":"PRIVATE_PAGE"}));
    let material =
        nexofolio_contracts::request::RequestMaterial::from_observation("private-sample", &raw)
            .unwrap();
    let at = chrono::Utc::now();
    let fields = vec![FieldFacts {
        location: FieldLocation::RequestBody,
        path: FieldPath(vec![PathSegment::Key("customerId".into())]),
        types: vec![ValueType::String],
        labels: vec![],
        differs_between_environments: false,
        declared: None,
        conflict: None,
    }];
    for (id, project_id, aliases) in [
        (target, access.project, vec![alias]),
        (foreign, ProjectId::new(), vec![]),
    ] {
        reader.data.put(EndpointFacts {
            summary: EndpointSummary {
                id,
                project_id,
                method: "POST".into(),
                path_template: "/orders".into(),
                external: false,
                declared: false,
                environments: vec![EnvironmentUsage {
                    environment_id: access.env,
                    calls: 2,
                    first_seen: at,
                    last_seen: at,
                }],
            },
            aliases,
            addresses: vec![],
            fields: fields.clone(),
            examples: vec![],
            requests: vec![material.clone()],
        });
    }
    reader
        .data
        .put_example(target, "private-sample", serde_json::to_value(raw).unwrap());
    let knowledge = Arc::new(Knowledge::new(InMemoryKnowledge::default(), reader.clone()));
    let round = knowledge
        .begin(
            access.project,
            Author::Person {
                user: access.member,
            },
        )
        .await
        .unwrap();
    let folder = nexofolio_common::FolderId::new();
    knowledge
        .apply(
            round,
            &[
                Command::PutFolder {
                    id: folder,
                    parent: None,
                    name: "订单".into(),
                    summary: Some("订单文档".into()),
                    position: 0,
                },
                Command::Place {
                    endpoint: target,
                    folder,
                },
                Command::PutEndpointNote {
                    endpoint: target,
                    name: Some("创建订单".into()),
                    purpose: Some("提交订单资料".into()),
                    context: EndpointContext {
                        description: Some("正式接口说明".into()),
                        input_hash: Some("PRIVATE_INPUT_HASH".into()),
                        ..Default::default()
                    },
                },
                Command::PutFieldNote {
                    endpoint: target,
                    path: "request_body:customerId".into(),
                    text: "客户的业务标识".into(),
                    values: vec!["PRIVATE_VALUE".into()],
                    context: FieldContext {
                        display_name: Some("客户编号".into()),
                        location: Some(FieldLocation::RequestBody),
                        evidence: vec![FieldEvidence {
                            kind: EvidenceKind::Observation,
                            text: "PRIVATE_EVIDENCE".into(),
                            endpoint: target,
                            field: "customerId".into(),
                            example: Some("private-sample".into()),
                            environment: Some(access.env),
                            options: vec![FieldOption {
                                value: "PRIVATE_OPTION".into(),
                                label: None,
                            }],
                            source: None,
                        }],
                        input_hash: Some("PRIVATE_FIELD_HASH".into()),
                        ..Default::default()
                    },
                },
            ],
        )
        .await
        .unwrap();
    knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let addresses = Arc::new(InMemoryAddresses::new());
    let private = crate::http::endpoints::routes(
        access.clone(),
        access.clone(),
        Arc::new(reader.clone()),
        knowledge.clone(),
    )
    .merge(crate::http::service_addresses::routes(
        access.clone(),
        access.clone(),
        addresses.clone(),
    ));
    let public = super::super::routes(
        "fixture".into(),
        access.clone(),
        Arc::new(reader.clone()),
        knowledge.clone(),
    );
    let config = crate::wiring::Config::from_lookup(|k| {
        (k == "DATABASE_URL").then(|| "postgres://unused@127.0.0.1/unused".into())
    })
    .unwrap();
    let app = crate::http::router_with_navigation(
        &config,
        access.clone(),
        access.clone(),
        tokio_util::sync::CancellationToken::new(),
        Some(private.merge(public)),
        Some(crate::mcp::Navigation::new(
            access.clone(),
            Arc::new(reader.clone()),
            knowledge,
        )),
    );
    Fixture {
        app,
        access,
        target,
        alias,
        foreign,
        reader,
        addresses,
    }
}

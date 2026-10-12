use super::*;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use nexofolio_access_contracts::*;
use nexofolio_common::{Error, TokenId, UserId};
use nexofolio_contracts::{
    endpoint::*,
    knowledge::*,
    observation::*,
    testing::{InMemoryEndpoints, sample_exchange},
};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Clone)]
pub(super) struct Reader {
    pub data: Arc<InMemoryEndpoints>,
    pub gets: Arc<AtomicUsize>,
}
#[async_trait]
impl EndpointReader for Reader {
    async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        self.data.list(p).await
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        self.data.get(id).await
    }
    async fn example(&self, id: EndpointId, e: &str) -> Result<Option<Value>, EndpointError> {
        self.data.example(id, e).await
    }
}
pub(super) struct Allow(pub ProjectId);
#[async_trait]
impl McpProjectAccess for Allow {
    async fn require_mcp_project(
        &self,
        _: &McpPrincipal,
        p: ProjectId,
    ) -> nexofolio_common::Result<()> {
        if p == self.0 {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
    async fn list_mcp_projects(
        &self,
        _: &McpPrincipal,
        _: u64,
        _: u32,
    ) -> nexofolio_common::Result<McpDiscoveryPage<McpProject>> {
        unreachable!()
    }
    async fn list_mcp_environments(
        &self,
        _: &McpPrincipal,
        _: ProjectId,
        _: u64,
        _: u32,
    ) -> nexofolio_common::Result<McpDiscoveryPage<McpEnvironment>> {
        unreachable!()
    }
}
pub(super) struct Fixture {
    pub nav: Navigation,
    pub project: ProjectId,
    pub env: EnvironmentId,
    pub other_env: EnvironmentId,
    pub root: FolderId,
    pub child: FolderId,
    pub target: EndpointId,
    pub related: EndpointId,
    pub unplaced: EndpointId,
    pub reader: Reader,
    pub knowledge: Arc<Knowledge<InMemoryKnowledge, Reader>>,
}
pub(super) fn field(path: Vec<PathSegment>) -> FieldFacts {
    FieldFacts {
        location: FieldLocation::RequestBody,
        path: FieldPath(path),
        types: vec![ValueType::String],
        labels: vec![],
        differs_between_environments: false,
        declared: None,
        conflict: None,
    }
}
pub(super) async fn setup() -> Fixture {
    let project = ProjectId::new();
    let env = EnvironmentId::new();
    let other_env = EnvironmentId::new();
    let root = FolderId::new();
    let child = FolderId::new();
    let sibling = FolderId::new();
    let target = EndpointId::new();
    let related = EndpointId::new();
    let unplaced = EndpointId::new();
    let reader = Reader {
        data: Arc::new(InMemoryEndpoints::default()),
        gets: Arc::new(AtomicUsize::new(0)),
    };
    let at = Utc.with_ymd_and_hms(2026, 10, 9, 0, 0, 0).unwrap();
    for (id, path, e) in [
        (target, "/orders", env),
        (related, "/customers", env),
        (unplaced, "/unclassified", other_env),
    ] {
        let mut raw = sample_exchange(project, e);
        if let Fact::Exchange(x) = &mut raw.fact {
            x.request.url = format!("https://synthetic.example{path}?a=1&a=2");
            x.request.headers = Some(Headers {
                complete: false,
                entries: vec![("X-Synthetic".into(), "sample".into())],
            });
            x.request.body = Body::Full {
                media_type: Some("application/json".into()),
                content: Content::Text("{}".into()),
            };
            x.response = Some(HttpResponse {
                status: 200,
                headers: Some(Headers {
                    complete: true,
                    entries: vec![],
                }),
                body: Body::Full {
                    media_type: Some("application/json".into()),
                    content: Content::Text(
                        json!({"data":"字".repeat(6000),"empty":null}).to_string(),
                    ),
                },
            });
        }
        raw.context = Some(json!({"page":{"title":"合成页面"},"operation":{"name":"合成按钮"}}));
        let material =
            nexofolio_contracts::request::RequestMaterial::from_observation("sample", &raw)
                .unwrap();
        let fields = if id == target {
            vec![
                field(vec![PathSegment::Key("customerId".into())]),
                field(vec![PathSegment::Key("group".into())]),
                field(vec![
                    PathSegment::Key("group".into()),
                    PathSegment::Key("name".into()),
                ]),
            ]
        } else {
            vec![]
        };
        reader.data.put(EndpointFacts {
            summary: EndpointSummary {
                id,
                project_id: project,
                method: "POST".into(),
                path_template: path.into(),
                external: false,
                declared: false,
                environments: vec![EnvironmentUsage {
                    environment_id: e,
                    calls: 1,
                    first_seen: at,
                    last_seen: at,
                }],
            },
            aliases: vec![],
            addresses: vec![],
            fields,
            examples: vec![],
            requests: vec![material],
        });
        reader
            .data
            .put_example(id, "sample", serde_json::to_value(raw).unwrap());
    }
    let knowledge = Arc::new(Knowledge::new(InMemoryKnowledge::default(), reader.clone()));
    let round = knowledge
        .begin(
            project,
            Author::Person {
                user: UserId::new(),
            },
        )
        .await
        .unwrap();
    knowledge
        .apply(
            round,
            &[
                Command::PutFolder {
                    id: root,
                    parent: None,
                    name: "订单".into(),
                    summary: Some("订单相关业务".into()),
                    position: 0,
                },
                Command::PutFolder {
                    id: child,
                    parent: Some(root),
                    name: "创建订单".into(),
                    summary: Some("提交订单".into()),
                    position: 0,
                },
                Command::PutFolder {
                    id: sibling,
                    parent: None,
                    name: "客户".into(),
                    summary: Some("客户资料".into()),
                    position: 1,
                },
                Command::Place {
                    endpoint: target,
                    folder: child,
                },
                Command::Place {
                    endpoint: related,
                    folder: sibling,
                },
                Command::PutEndpointNote {
                    endpoint: target,
                    name: Some("创建订单".into()),
                    purpose: Some("创建客户订单".into()),
                    context: Default::default(),
                },
                Command::PutEndpointNote {
                    endpoint: related,
                    name: Some("客户列表查询".into()),
                    purpose: Some("查询客户身份".into()),
                    context: Default::default(),
                },
                Command::PutFieldNote {
                    endpoint: target,
                    path: "request_body:customerId".into(),
                    text: "客户 ID，可从客户列表查询接口获得。".into(),
                    values: vec![],
                    context: FieldContext {
                        location: Some(FieldLocation::RequestBody),
                        field_path: Some("customerId".into()),
                        display_name: Some("客户编号".into()),
                        evidence: vec![FieldEvidence {
                            kind: EvidenceKind::RelatedInterface,
                            text: "合成关联证据".into(),
                            endpoint: related,
                            field: "data[].id".into(),
                            example: None,
                            environment: Some(env),
                            options: vec![],
                            source: None,
                        }],
                        ..Default::default()
                    },
                },
                Command::PutLink {
                    from: related,
                    from_field: Some("data[].id".into()),
                    to: target,
                    to_field: Some("customerId".into()),
                    relation: Relation::ParameterSource,
                    evidence: "合成值对应，不代表硬性前置".into(),
                    context: LinkContext {
                        from_location: Some(FieldLocation::ResponseBody { status: 200 }),
                        to_location: Some(FieldLocation::RequestBody),
                        environment: Some(env),
                        confirmation: Confirmation::Observed,
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
    let nav = Navigation::new(
        Arc::new(Allow(project)),
        Arc::new(reader.clone()),
        knowledge.clone(),
    );
    Fixture {
        nav,
        project,
        env,
        other_env,
        root,
        child,
        target,
        related,
        unplaced,
        reader,
        knowledge,
    }
}
pub(super) async fn call(
    f: &Fixture,
    tool: &str,
    mut args: Value,
) -> Result<Value, super::super::navigation::ReadError> {
    args["project_id"] = json!(f.project);
    f.nav
        .call(
            &McpPrincipal {
                user_id: UserId::new(),
                token_id: TokenId::new(),
            },
            tool,
            args,
        )
        .await
}

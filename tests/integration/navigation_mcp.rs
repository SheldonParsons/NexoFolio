//! Actual TCP + MCP protocol + isolated PostgreSQL knowledge/history/access, fixed model only.
#[path = "../../modules/curate/core/tests/stage1/mod.rs"]
mod stage1;
use async_trait::async_trait;
use nexofolio_access_adapter::Postgres;
use nexofolio_backend::{http, mcp::Navigation, wiring::Config};
use nexofolio_common::{EndpointId, ProjectId, Secret, UserId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::*;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Facts(Vec<EndpointFacts>);
#[async_trait]
impl EndpointReader for Facts {
    async fn list(&self, project: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(self
            .0
            .iter()
            .filter(|f| f.summary.project_id == project)
            .map(|f| f.summary.clone())
            .collect())
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok(self.0.iter().find(|f| f.summary.id == id).cloned())
    }
    async fn example(&self, _: EndpointId, _: &str) -> Result<Option<Value>, EndpointError> {
        panic!("MCP must not read raw samples")
    }
}
struct Fixed;
#[async_trait]
impl Completions for Fixed {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        if let Some(start) = prompt.find("字段和可核查的证据：") {
            let data: Value =
                serde_json::Deserializer::from_str(&prompt[start + "字段和可核查的证据：".len()..])
                    .into_iter::<Value>()
                    .next()
                    .unwrap()
                    .unwrap();
            return Ok(json!({"fields":data.as_array().unwrap().iter().map(|f|json!({"key":f["key"],
                "display_name":"基础推测","description":"按所在对象理解；未知业务规则待确认","details":[],"unknowns":[]})).collect::<Vec<_>>()}).to_string());
        }
        Ok(json!({"name":"保存并提交技术咨询","purpose":"业务端保存并提交咨询及商品条目",
            "description":"输入项目、商品及咨询信息；项目 ID 须已知或有发现来源。认证来源未闭合。仅保存响应不证明业务创建成功。",
            "unknowns":["供销云登录来源未确认"]}).to_string())
    }
}
fn uuid(id: impl std::fmt::Display) -> uuid::Uuid {
    id.to_string().parse().unwrap()
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn actual_mcp_navigation_with_postgres_saved_knowledge_and_project_grants() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let f = stage1::fixture();
    let project = f.project();
    let user = UserId::new();
    let access = Postgres::new(&url, 4, Duration::from_secs(5)).unwrap();
    access.migrate().await.unwrap();
    let store = PostgresKnowledge::new(&url, 4, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    let pool = sqlx::PgPool::connect(url.expose()).await.unwrap();
    sqlx::query("INSERT INTO access.users(id,instance,external_id,account,display_name,enabled,grants_synced) VALUES($1,'fixture','fixture','fixture','fixture',true,true)")
        .bind(uuid(user)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO access.projects(id,instance,external_id,name,status,last_sync_generation,last_synced_at) VALUES($1,'fixture','stage1','隔离项目','active',1,now())")
        .bind(uuid(project)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO access.user_project_access(user_id,project_id) VALUES($1,$2)")
        .bind(uuid(user))
        .bind(uuid(project))
        .execute(&pool)
        .await
        .unwrap();
    let (token_id, token) = access.issue_mcp_token(user, 7).await.unwrap();
    // A constructed endpoint exercises multi-environment selection and nested
    // sensitive legacy knowledge through the actual MCP response boundary.
    let mut regression = f.endpoint("saveAndSubmitConsultTechnology").clone();
    regression.summary.id = EndpointId::new();
    regression.summary.path_template = "/review-regression-only".into();
    let mut extra_env = regression.summary.environments[0].clone();
    extra_env.environment_id = nexofolio_common::EnvironmentId::new();
    regression.summary.environments.push(extra_env);
    regression.fields = vec![FieldFacts {
        location: FieldLocation::RequestBody,
        path: FieldPath(vec![PathSegment::Key("password".into())]),
        types: vec![ValueType::String],
        labels: vec![],
        differs_between_environments: false,
        declared: None,
        conflict: None,
    }];
    let mut reader_facts = f.facts.clone();
    reader_facts.push(regression.clone());
    let reader = Facts(reader_facts);
    let knowledge = Arc::new(Knowledge::new(store.clone(), reader.clone()));
    let pass = Describe::new(Fixed, "fixed-fixture".into());
    let create = f.endpoint("saveAndSubmitConsultTechnology");
    let page = f.endpoint("manager/consultTechnology/page");
    let summaries: Vec<_> = f.facts.iter().map(|f| f.summary.clone()).collect();
    let links = navigation_links(&f.facts, &f.signals);
    let round = knowledge.begin(project, pass.author()).await.unwrap();
    let auth = f
        .authentication
        .iter()
        .filter(|(id, _)| *id == create.summary.id)
        .flat_map(|(_, a)| a.clone())
        .collect();
    let input = endpoint_input(create, &f.signals, &summaries, auth);
    let description = pass
        .endpoint_with_input(create, &input, None)
        .await
        .unwrap()
        .unwrap();
    let fields = pass
        .fields(
            page,
            &EndpointKnowledge::default(),
            &field_inputs(page, &f.signals, &summaries),
        )
        .await
        .unwrap();
    let mut commands = vec![description];
    commands.extend(fields);
    commands.extend(relation_commands(&links, &[], None));
    let dummy = "DUMMY_MCP_REVIEW_ONLY";
    commands.extend([
        Command::PutEndpointNote {
            endpoint: regression.summary.id,
            name: None,
            purpose: None,
            context: EndpointContext {
                authentication: input.authentication.clone(),
                ..Default::default()
            },
        },
        Command::PutFieldNote {
            endpoint: regression.summary.id,
            path: "request_body:password".into(),
            text: dummy.into(),
            values: vec![dummy.into()],
            context: FieldContext {
                location: Some(FieldLocation::RequestBody),
                field_path: Some("password".into()),
                display_name: Some(dummy.into()),
                meaning: FieldMeaning {
                    details: vec![dummy.into()],
                    ..Default::default()
                },
                ..Default::default()
            },
        },
        Command::PutLink {
            from: create.summary.id,
            from_field: Some("access_token".into()),
            to: regression.summary.id,
            to_field: Some("password".into()),
            relation: Relation::AuthenticationUse,
            evidence: dummy.into(),
            context: LinkContext {
                environment: Some(f.environment()),
                confirmation: Confirmation::Observed,
                ..Default::default()
            },
        },
    ]);
    knowledge.apply(round, &commands).await.unwrap();
    knowledge
        .finish(round, RoundState::Done, Some("隔离固定模型验证".into()))
        .await
        .unwrap();
    let saved = knowledge.endpoint(page.summary.id).await.unwrap();
    assert!(
        saved
            .fields
            .iter()
            .any(|n| n.context.display_name.as_deref() == Some("咨询子单号"))
    );
    assert!(relation_commands(&links, &knowledge.links(project).await.unwrap(), None).is_empty());
    assert!(!description_pending(
        &input,
        knowledge
            .endpoint(create.summary.id)
            .await
            .unwrap()
            .note
            .as_ref()
    ));
    assert!(pending_fields(page, &saved, field_inputs(page, &f.signals, &summaries)).is_empty());
    // Manual protection and transactional history include the new layered context.
    let manual = knowledge
        .begin(project, Author::Person { user })
        .await
        .unwrap();
    let original = saved
        .fields
        .iter()
        .find(|n| n.context.display_name.as_deref() == Some("咨询子单号"))
        .unwrap();
    let mut context = original.context.clone();
    context.display_name = Some("人工确认名".into());
    knowledge
        .apply(
            manual,
            &[Command::PutFieldNote {
                endpoint: page.summary.id,
                path: original.path.clone(),
                text: "人工确认说明".into(),
                values: vec![],
                context,
            }],
        )
        .await
        .unwrap();
    knowledge
        .finish(manual, RoundState::Done, None)
        .await
        .unwrap();
    let now = knowledge.endpoint(page.summary.id).await.unwrap();
    assert!(pending_fields(page, &now, field_inputs(page, &f.signals, &summaries)).is_empty());
    knowledge.roll_back(manual).await.unwrap();
    assert_eq!(knowledge.endpoint(page.summary.id).await.unwrap(), saved);

    let config = Config::from_lookup(|key| {
        if key == "DATABASE_URL" {
            Some("postgres://unused@localhost/unused".into())
        } else {
            None
        }
    })
    .unwrap();
    let stop = CancellationToken::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/mcp", listener.local_addr().unwrap());
    let router = http::router_with_navigation(
        &config,
        Arc::new(access.clone()),
        Arc::new(access.clone()),
        stop.clone(),
        None,
        Some(Navigation::new(
            Arc::new(access.clone()),
            Arc::new(reader),
            knowledge.clone(),
        )),
    );
    let server = tokio::spawn(http::serve(
        listener,
        router,
        stop.clone(),
        Duration::from_secs(1),
    ));
    let client = reqwest::Client::new();
    let invoke = |id, method: &str, params: Value| {
        client
            .post(&base)
            .bearer_auth(token.expose())
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25")
            .json(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
    };
    assert_eq!(client.post(&base).send().await.unwrap().status(), 401);
    let init:Value=invoke(1,"initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"stage1-real-protocol","version":"1"}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "nexofolio-knowledge");
    let tools: Value = invoke(2, "tools/list", json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 7);
    let search:Value=invoke(3,"tools/call",json!({"name":"search_interfaces","arguments":{"project_id":project,"query":"创建并提交技术咨询","limit":1}}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(
        search["result"]["structuredContent"]["items"][0]["endpoint_id"],
        json!(create.summary.id)
    );
    let read:Value=invoke(4,"tools/call",json!({"name":"read_interface","arguments":{"project_id":project,"endpoint_id":page.summary.id,"limit":2}}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(
        read["result"]["structuredContent"]["fields"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(read["result"]["structuredContent"]["fields_has_more"], true);
    let read_next:Value=invoke(40,"tools/call",json!({"name":"read_interface","arguments":{"project_id":project,"endpoint_id":page.summary.id,"limit":2,"field_offset":2}})).send().await.unwrap().json().await.unwrap();
    assert_ne!(
        read["result"]["structuredContent"]["fields"][0]["key"],
        read_next["result"]["structuredContent"]["fields"][0]["key"]
    );
    let foreign:Value=invoke(41,"tools/call",json!({"name":"read_interface","arguments":{"project_id":project,"endpoint_id":EndpointId::new()}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(
        foreign["result"]["structuredContent"]["error"],
        "ENDPOINT_NOT_IN_PROJECT"
    );

    let item_offset = page
        .fields
        .iter()
        .position(|field| field.path.to_string() == "data.records[].itemCode")
        .unwrap();
    let item_read:Value=invoke(42,"tools/call",json!({"name":"read_interface","arguments":{"project_id":project,"endpoint_id":page.summary.id,"field_offset":item_offset,"limit":1}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(
        item_read["result"]["structuredContent"]["fields"][0]["knowledge"]["context"]["display_name"],
        "咨询子单号"
    );
    let amount_offset = page
        .fields
        .iter()
        .position(|field| field.path.to_string() == "data.records[].estimatedAmount")
        .unwrap();
    let amount_read:Value=invoke(43,"tools/call",json!({"name":"read_interface","arguments":{"project_id":project,"endpoint_id":page.summary.id,"field_offset":amount_offset,"limit":1}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(
        amount_read["result"]["structuredContent"]["fields"][0]["knowledge"]["context"]["meaning"]
            ["unit"],
        "万"
    );
    assert_eq!(
        amount_read["result"]["structuredContent"]["fields"][0]["knowledge"]["context"]["display_name_confirmation"],
        "direct"
    );
    assert_eq!(
        amount_read["result"]["structuredContent"]["fields"][0]["knowledge"]["context"]["meaning"]
            ["unit_confirmation"],
        "direct"
    );
    assert_eq!(
        amount_read["result"]["structuredContent"]["fields"][0]["knowledge"]["context"]["meaning"]
            ["confirmation"],
        "candidate"
    );
    let trace:Value=invoke(5,"tools/call",json!({"name":"trace_dependencies","arguments":{"project_id":project,"endpoint_id":create.summary.id,"environment_id":f.environment(),"authenticated":true,
        "known_inputs":[{"endpoint":create.summary.id,"location":{"in":"request_body"},"path":"opptyProjectId"}],"max_steps":100}}))
        .send().await.unwrap().json().await.unwrap();
    let text = trace.to_string();
    assert!(text.contains("input_reused"));
    assert!(text.contains("authentication_reused"));
    assert!(text.contains("input_gap"));
    let trace_inferred:Value=invoke(44,"tools/call",json!({"name":"trace_dependencies","arguments":{"project_id":project,"endpoint_id":create.summary.id,"authenticated":true,
        "known_inputs":[{"endpoint":create.summary.id,"location":{"in":"request_body"},"path":"opptyProjectId"}],"max_steps":100}}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(
        trace_inferred["result"]["structuredContent"], trace["result"]["structuredContent"],
        "omission must load the same scoped upstream graph"
    );
    let trace_gap:Value=invoke(45,"tools/call",json!({"name":"trace_dependencies","arguments":{"project_id":project,"endpoint_id":create.summary.id}}))
        .send().await.unwrap().json().await.unwrap();
    assert!(trace_gap.to_string().contains("authentication_gap"));
    let ambiguous:Value=invoke(46,"tools/call",json!({"name":"trace_dependencies","arguments":{"project_id":project,"endpoint_id":regression.summary.id}}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(
        ambiguous["result"]["structuredContent"]["steps"][0]["state"],
        "environment_required"
    );
    assert_eq!(
        ambiguous["result"]["structuredContent"]["available_environments"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let sensitive_read:Value=invoke(47,"tools/call",json!({"name":"read_interface","arguments":{"project_id":project,"endpoint_id":regression.summary.id}}))
        .send().await.unwrap().json().await.unwrap();
    assert!(sensitive_read.to_string().contains(dummy));
    let meaning = &sensitive_read["result"]["structuredContent"]["fields"][0]["knowledge"]["context"]
        ["meaning"];
    assert_eq!(meaning["details"][0], dummy);
    assert_eq!(
        sensitive_read["result"]["structuredContent"]["links"][0]["evidence"],
        dummy
    );
    let denied:Value=invoke(6,"tools/call",json!({"name":"read_interface","arguments":{"project_id":ProjectId::new(),"endpoint_id":page.summary.id}}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(
        denied["result"]["structuredContent"]["error"],
        "PROJECT_ACCESS_DENIED"
    );
    let bounds:Value=invoke(7,"tools/call",json!({"name":"read_interface","arguments":{"project_id":project,"endpoint_id":page.summary.id,"limit":51}}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(
        bounds["result"]["structuredContent"]["error"],
        "INVALID_BOUNDS"
    );
    // Record exact non-sensitive protocol outputs for the handoff.
    let evidence = json!({"source":"isolated postgres + sanitized observations + fixed model; no real business execution",
        "initialize":init,"tools_list":tools,"search":search,"read_page":read,"read_item_code":item_read,"read_amount":amount_read,"read_next_page":read_next,"foreign_endpoint":foreign,"trace":trace,"authorization_denied":denied,"bounds_rejected":bounds,
        "regressions":{"inferred_environment":trace_inferred,"default_auth_gap":trace_gap,"multiple_environments":ambiguous,"sensitive_read":sensitive_read},
        "measured":{"endpoints":f.facts.len(),"signals":f.signals.len(),"relations":links.len(),"saved_field_notes":saved.fields.len(),"additional_constructed_endpoints":1}});
    let serialized = evidence.to_string();
    assert!(!serialized.contains(token.expose()));
    assert!(!serialized.contains("fixture-only-unmatched-business-credential"));
    if let Ok(path) = std::env::var("STAGE1_MCP_EVIDENCE_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
    // Effective grants are rechecked per call, not cached at initialize.
    sqlx::query("DELETE FROM access.user_project_access WHERE user_id=$1")
        .bind(uuid(user))
        .execute(&pool)
        .await
        .unwrap();
    let denied: Value = invoke(
        8,
        "tools/call",
        json!({"name":"search_interfaces","arguments":{"project_id":project}}),
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(
        denied["result"]["structuredContent"]["error"],
        "PROJECT_ACCESS_DENIED"
    );
    access.revoke_mcp_token(token_id).await.unwrap();
    assert_eq!(
        invoke(9, "tools/list", json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    stop.cancel();
    assert!(server.await.unwrap().is_ok());
    knowledge.roll_back(round).await.unwrap();
    assert!(
        knowledge
            .endpoint(page.summary.id)
            .await
            .unwrap()
            .fields
            .is_empty()
    );
    assert!(knowledge.links(project).await.unwrap().is_empty());
    pool.close().await;
    store.close().await;
    access.close().await;
}

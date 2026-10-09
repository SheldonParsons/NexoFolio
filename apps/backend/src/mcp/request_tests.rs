//! Synthetic request reconstruction through the authorized MCP application boundary.
use super::Navigation;
use async_trait::async_trait;
use nexofolio_access_contracts::{McpPrincipal, McpProjectAccess};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId, TokenId, UserId};
use nexofolio_contracts::{endpoint::*, observation::*, testing::sample_exchange};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Clone)]
struct Captured {
    facts: EndpointFacts,
    raw: Value,
}
#[async_trait]
impl EndpointReader for Captured {
    async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(if p == self.facts.summary.project_id {
            vec![self.facts.summary.clone()]
        } else {
            vec![]
        })
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok((id == self.facts.summary.id).then(|| self.facts.clone()))
    }
    async fn example(&self, id: EndpointId, sample: &str) -> Result<Option<Value>, EndpointError> {
        Ok((id == self.facts.summary.id && sample == "fixture").then(|| self.raw.clone()))
    }
}
struct Allow;
#[async_trait]
impl McpProjectAccess for Allow {
    async fn list_mcp_projects(
        &self,
        _: &McpPrincipal,
        _: u64,
        _: u32,
    ) -> nexofolio_common::Result<
        nexofolio_access_contracts::McpDiscoveryPage<nexofolio_access_contracts::McpProject>,
    > {
        panic!("request-material tests must not use discovery")
    }
    async fn list_mcp_environments(
        &self,
        _: &McpPrincipal,
        _: ProjectId,
        _: u64,
        _: u32,
    ) -> nexofolio_common::Result<
        nexofolio_access_contracts::McpDiscoveryPage<nexofolio_access_contracts::McpEnvironment>,
    > {
        panic!("request-material tests must not use discovery")
    }
    async fn require_mcp_project(
        &self,
        _: &McpPrincipal,
        _: ProjectId,
    ) -> nexofolio_common::Result<()> {
        Ok(())
    }
}
#[tokio::test]
async fn material_chunks_reassemble_original_headers_query_root_and_auth_gaps() {
    let project = ProjectId::new();
    let endpoint = EndpointId::new();
    let env = EnvironmentId::new();
    let mut raw = sample_exchange(project, env);
    let Fact::Exchange(exchange) = &mut raw.fact else {
        unreachable!()
    };
    exchange.request.method = "POST".into();
    exchange.request.url = "https://fixture.test/api?q=1&q=2".into();
    exchange.request.headers = Some(Headers {
        complete: false,
        entries: vec![
            ("Authorization".into(), "Bearer synthetic".into()),
            ("X-Label".into(), "重复".into()),
            ("X-Label".into(), "两个".into()),
        ],
    });
    exchange.request.body = Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text(json!({"data":"字".repeat(18000)}).to_string()),
    };
    let material =
        nexofolio_contracts::request::RequestMaterial::from_observation("fixture", &raw).unwrap();
    let facts = EndpointFacts {
        summary: EndpointSummary {
            id: endpoint,
            project_id: project,
            method: "POST".into(),
            path_template: "/api".into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        aliases: vec![],
        addresses: vec![],
        fields: vec![],
        examples: vec![],
        requests: vec![material],
    };
    let captured = Captured {
        facts,
        raw: serde_json::to_value(raw).unwrap(),
    };
    let navigation = Navigation::new(
        Arc::new(Allow),
        Arc::new(captured.clone()),
        Arc::new(Knowledge::new(InMemoryKnowledge::new(), captured)),
    );
    let principal = McpPrincipal {
        user_id: UserId::new(),
        token_id: TokenId::new(),
    };
    let read = navigation
        .call(
            &principal,
            "read_interface",
            json!({"project_id":project,"endpoint_id":endpoint}),
        )
        .await
        .unwrap();
    assert_eq!(read["request_material_total"], 1);
    let mut offset = 0;
    let mut collected = String::new();
    let mut calls = 0;
    loop {
        let part=navigation.call(&principal,"read_request_material",json!({"project_id":project,"endpoint_id":endpoint,"example_id":"fixture","content_offset":offset})).await.unwrap();
        collected.push_str(part["data"].as_str().unwrap());
        calls += 1;
        offset = part["next_offset"].as_u64().unwrap();
        if part["has_more"] == false {
            break;
        }
    }
    assert!(calls > 1);
    assert_eq!(navigation.call(&principal,"read_request_material",json!({"project_id":project,"endpoint_id":endpoint,"example_id":"fixture","content_offset":1,"source_record":{"batch_id":uuid::Uuid::new_v4(),"record_id":uuid::Uuid::new_v4()}})).await.unwrap_err(),"SOURCE_CHANGED_RESTART_READ");
    let result: Value = serde_json::from_str(&collected).unwrap();
    assert_eq!(
        result["request"]["headers"]["entries"][0][1],
        "Bearer synthetic"
    );
    assert_eq!(
        result["request"]["headers"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(result["request"]["url"], "https://fixture.test/api?q=1&q=2");
    assert_eq!(
        result["body_root"]["data"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        18000
    );
    assert_eq!(
        result["authentication"][0]["current_validity"],
        "unknown_historical_observation"
    );
    assert!(result["authentication"][0]["acquisition_source"].is_null());
    assert!(
        navigation
            .call(
                &principal,
                "read_request_material",
                json!({"project_id":ProjectId::new(),"endpoint_id":endpoint,"example_id":"fixture"})
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn field_values_can_be_paged_and_read_in_full_through_mcp() {
    use nexofolio_contracts::knowledge::{CandidateState, KnowledgeCandidates};
    let project = ProjectId::new();
    let endpoint = EndpointId::new();
    let env = EnvironmentId::new();
    let mut raw = sample_exchange(project, env);
    if let Fact::Exchange(e) = &mut raw.fact {
        e.request.body = Body::Full {
            media_type: Some("application/json".into()),
            content: Content::Text(json!({"value":"字".repeat(18000)}).to_string()),
        };
    }
    let facts = EndpointFacts {
        summary: EndpointSummary {
            id: endpoint,
            project_id: project,
            method: "POST".into(),
            path_template: "/fixture".into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        aliases: vec![],
        addresses: vec![],
        examples: vec![],
        requests: vec![],
        fields: vec![FieldFacts {
            location: FieldLocation::RequestBody,
            path: FieldPath(vec![PathSegment::Key("value".into())]),
            types: vec![ValueType::String],
            labels: vec![],
            differs_between_environments: false,
            declared: None,
            conflict: None,
        }],
    };
    let raw = serde_json::to_value(raw).unwrap();
    let archive =
        nexofolio_observe::archive_values(&facts, &[("fixture".into(), Some(raw.clone()))]);
    let captured = Captured { facts, raw };
    let knowledge = Arc::new(Knowledge::new(InMemoryKnowledge::new(), captured.clone()));
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let owner = uuid::Uuid::new_v4();
    knowledge
        .checkpoint_candidate(
            project,
            candidate.id,
            CandidateState::Running,
            json!({"run_id":owner}),
        )
        .await
        .unwrap();
    knowledge
        .freeze_candidate_values(project, candidate.id, owner, &archive)
        .await
        .unwrap();
    let nav = Navigation::new(Arc::new(Allow), Arc::new(captured), knowledge);
    let principal = McpPrincipal {
        user_id: UserId::new(),
        token_id: TokenId::new(),
    };
    let page=nav.call(&principal,"read_field_values",json!({"project_id":project,"endpoint_id":endpoint,"archive_id":candidate.id,"field_key":"request_body:value","environment_id":env})).await.unwrap();
    assert_eq!(page["summary"]["total"], 1);
    assert_eq!(page["items"][0]["preview_truncated"], true);
    let id = page["items"][0]["id"].as_str().unwrap();
    let mut offset = 0;
    let mut complete = String::new();
    let mut pages = 0;
    loop {
        let part=nav.call(&principal,"read_field_values",json!({"project_id":project,"endpoint_id":endpoint,"archive_id":candidate.id,"field_key":"request_body:value","value_id":id,"content_offset":offset})).await.unwrap();
        complete.push_str(part["data"].as_str().unwrap());
        pages += 1;
        offset = part["next_offset"].as_u64().unwrap();
        if part["has_more"] == false {
            break;
        }
    }
    assert!(pages > 1);
    assert_eq!(
        serde_json::from_str::<String>(&complete)
            .unwrap()
            .chars()
            .count(),
        18000
    );
}

//! What each pass turns a model reply into. The model is a canned string, so
//! these tests pin the mapping from reply to `Command` and nothing else.

use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::{
    endpoint::{EndpointFacts, EndpointSummary},
    knowledge::{Author, Command},
};
use nexofolio_curate::{CompletionError, Completions, Describe, Organize};

struct Canned(String);

#[async_trait]
impl Completions for Canned {
    async fn complete(&self, _prompt: &str) -> Result<String, CompletionError> {
        Ok(self.0.clone())
    }
}

struct Broken;

#[async_trait]
impl Completions for Broken {
    async fn complete(&self, _prompt: &str) -> Result<String, CompletionError> {
        Err(CompletionError("model is down".into()))
    }
}

fn summary(project: ProjectId, method: &str, path: &str) -> EndpointSummary {
    EndpointSummary {
        id: EndpointId::new(),
        project_id: project,
        method: method.into(),
        path_template: path.into(),
        declared: false,
        external: false,
        environments: vec![],
    }
}

fn facts(summary: EndpointSummary) -> EndpointFacts {
    EndpointFacts {
        summary,
        examples: vec![],
        fields: vec![],
        addresses: vec![],
        aliases: vec![],
    }
}

#[tokio::test]
async fn describe_writes_one_note_from_a_fenced_reply() {
    let project = ProjectId::new();
    let s = summary(project, "POST", "/api/orders/create");
    let endpoint = s.id;
    let pass = Describe::new(
        Canned(
            "```json\n{\"name\": \"创建订单\", \"purpose\": \"提交新订单并返回订单号\"}\n```"
                .into(),
        ),
        "test-model".into(),
    );

    let command = pass.endpoint(&facts(s)).await.unwrap();

    assert_eq!(
        command,
        Some(Command::PutEndpointNote {
            endpoint,
            name: Some("创建订单".into()),
            purpose: Some("提交新订单并返回订单号".into()),
        })
    );
}

#[tokio::test]
async fn describe_writes_nothing_when_the_model_returns_blanks() {
    let project = ProjectId::new();
    let pass = Describe::new(
        Canned("{\"name\": \"  \", \"purpose\": \"\"}".into()),
        "test-model".into(),
    );

    let command = pass
        .endpoint(&facts(summary(project, "GET", "/api/ping")))
        .await
        .unwrap();

    assert_eq!(command, None);
}

#[tokio::test]
async fn describe_reports_a_reply_with_no_json() {
    let project = ProjectId::new();
    let pass = Describe::new(Canned("抱歉，我无法回答。".into()), "test-model".into());

    assert!(
        pass.endpoint(&facts(summary(project, "GET", "/api/ping")))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn describe_names_the_pass_and_the_model_as_author() {
    let pass = Describe::new(Canned("{}".into()), "deepseek-v4-pro".into());
    assert_eq!(
        pass.author(),
        Author::Curate {
            pass: "describe".into(),
            model: Some("deepseek-v4-pro".into()),
        }
    );
}

#[tokio::test]
async fn describe_passes_the_model_failure_through() {
    let project = ProjectId::new();
    let pass = Describe::new(Broken, "test-model".into());
    let error = pass
        .endpoint(&facts(summary(project, "GET", "/api/ping")))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("model is down"));
}

#[tokio::test]
async fn organize_creates_folders_and_places_matching_endpoints() {
    let project = ProjectId::new();
    let users = summary(project, "GET", "/api/users/list");
    let orders = summary(project, "POST", "/api/orders/create");
    let stray = summary(project, "GET", "/api/health");
    let (users_id, orders_id, stray_id) = (users.id, orders.id, stray.id);

    let pass = Organize::new(
        Canned(
            r#"[
              {"name": "用户", "summary": "用户查询", "match": ["/users/"]},
              {"name": "订单", "summary": "下单与查单", "match": ["/orders/"]}
            ]"#
            .into(),
        ),
        "test-model".into(),
    );

    let commands = pass.project(&[users, orders, stray]).await.unwrap();

    // Two folders, and only the two endpoints whose paths matched are placed.
    let folders: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::PutFolder { id, name, position, .. } => Some((*id, name.clone(), *position)),
            _ => None,
        })
        .collect();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0].1, "用户");
    assert_eq!(folders[0].2, 0);
    assert_eq!(folders[1].1, "订单");
    assert_eq!(folders[1].2, 1);

    let placements: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::Place { endpoint, folder } => Some((*endpoint, *folder)),
            _ => None,
        })
        .collect();
    assert_eq!(placements.len(), 2);
    assert!(placements.contains(&(users_id, folders[0].0)));
    assert!(placements.contains(&(orders_id, folders[1].0)));
    assert!(!placements.iter().any(|(e, _)| *e == stray_id));
}

#[tokio::test]
async fn organize_places_an_endpoint_in_the_first_folder_that_claims_it() {
    let project = ProjectId::new();
    let shared = summary(project, "GET", "/api/users/orders");
    let shared_id = shared.id;

    let pass = Organize::new(
        Canned(
            r#"[
              {"name": "用户", "summary": "用户", "match": ["/users/"]},
              {"name": "订单", "summary": "订单", "match": ["/orders"]}
            ]"#
            .into(),
        ),
        "test-model".into(),
    );

    let commands = pass.project(&[shared]).await.unwrap();
    let placements: Vec<_> = commands
        .iter()
        .filter(|c| matches!(c, Command::Place { .. }))
        .collect();

    // The tree is a real tree: one endpoint, one folder.
    assert_eq!(placements.len(), 1);
    assert!(matches!(
        placements[0],
        Command::Place { endpoint, .. } if *endpoint == shared_id
    ));
}

#[tokio::test]
async fn organize_does_not_call_the_model_for_an_empty_project() {
    let pass = Organize::new(Broken, "test-model".into());
    assert_eq!(pass.project(&[]).await.unwrap(), vec![]);
}

#[tokio::test]
async fn organize_names_the_pass_and_the_model_as_author() {
    let pass = Organize::new(Canned("[]".into()), "deepseek-v4-pro".into());
    assert_eq!(
        pass.author(),
        Author::Curate {
            pass: "organize".into(),
            model: Some("deepseek-v4-pro".into()),
        }
    );
}

#[tokio::test]
async fn organize_reports_a_reply_that_is_not_the_shape_asked_for() {
    let project = ProjectId::new();
    let pass = Organize::new(
        Canned("[{\"name\": \"用户\"}]".into()),
        "test-model".into(),
    );
    assert!(
        pass.project(&[summary(project, "GET", "/api/users/list")])
            .await
            .is_err()
    );
}

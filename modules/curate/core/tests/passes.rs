//! What each pass turns a model reply into. The model is a canned string, so
//! these tests pin the mapping from reply to `Command` and nothing else.

use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::{
    endpoint::{EndpointFacts, EndpointSummary},
    knowledge::{Author, Command},
};
use nexofolio_curate::{CompletionError, Completions, Describe};

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
        requests: vec![],
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

    assert!(matches!(command, Some(Command::PutEndpointNote {
        endpoint: id, name: Some(name), purpose: Some(purpose), context
    }) if id == endpoint && name == "创建订单" && purpose == "提交新订单并返回订单号"
        && context.recipe.as_deref() == Some("endpoint-knowledge-v2")));
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

#[test]
fn workflow_orders_passes_by_their_dependencies() {
    use nexofolio_curate::{Pass, workflow};
    assert_eq!(
        workflow(),
        vec![Pass::Describe, Pass::Fields, Pass::Organize]
    );
    assert_eq!(Pass::Organize.dependencies(), &[Pass::Fields]);
}

#[test]
fn description_reuse_ignores_call_counts_but_detects_new_structures_and_keeps_human_words() {
    use nexofolio_contracts::endpoint::{ExampleSummary, ServiceAddress};
    use nexofolio_contracts::knowledge::EndpointNote;
    let project = ProjectId::new();
    let mut endpoint = facts(summary(project, "GET", "/api/users"));
    let mut note = EndpointNote {
        endpoint: endpoint.summary.id,
        name: Some("用户".into()),
        purpose: Some("查询用户".into()),
        author: Author::Curate {
            pass: "describe".into(),
            model: None,
        },
        at: "2026-09-30T00:00:00Z".parse().unwrap(),

        context: Default::default(),
    };
    let pass = Describe::new(Broken, "test".into());
    assert!(!pass.needs_update(&endpoint, Some(&note)));
    endpoint.examples.push(ExampleSummary {
        structure_id: String::new(),
        retained_samples: 1,
        id: "structure".into(),
        environment_id: nexofolio_common::EnvironmentId::new(),
        address: ServiceAddress::parse("https://example.test").unwrap(),
        status: Some(200),
        calls: 1000,
        first_seen: "2026-09-29T00:00:00Z".parse().unwrap(),
        last_seen: "2026-09-30T12:00:00Z".parse().unwrap(),
    });
    assert!(!pass.needs_update(&endpoint, Some(&note)));
    endpoint.examples[0].first_seen = "2026-09-30T01:00:00Z".parse().unwrap();
    assert!(pass.needs_update(&endpoint, Some(&note)));
    note.author = Author::Person {
        user: nexofolio_common::UserId::new(),
    };
    assert!(!pass.needs_update(&endpoint, Some(&note)));
    assert!(pass.needs_update(&endpoint, None));
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

//! Refresh and round rollback must use the same qualified identity as inference/SQL.
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::testing::InMemoryEndpoints;
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;

#[tokio::test]
async fn field_locations_and_legacy_links_survive_round_update_and_rollback() {
    let project = ProjectId::new();
    let from = EndpointId::new();
    let to = EndpointId::new();
    let endpoints = InMemoryEndpoints::new();
    for id in [from, to] {
        endpoints.put(EndpointFacts {
            requests: vec![],
            summary: EndpointSummary {
                id,
                project_id: project,
                method: "POST".into(),
                path_template: "/test".into(),
                external: false,
                declared: false,
                environments: vec![],
            },
            fields: vec![],
            aliases: vec![],
            addresses: vec![],
            examples: vec![],
        });
    }
    let knowledge = Knowledge::new(InMemoryKnowledge::new(), endpoints);
    let author = Author::Curate {
        pass: "navigation".into(),
        model: None,
    };
    let env = EnvironmentId::new();
    let put = |from_location, to_location, environment, text: &str| Command::PutLink {
        from,
        from_field: Some("data.id".into()),
        to,
        to_field: Some("id".into()),
        relation: Relation::ParameterSource,
        evidence: text.into(),
        context: LinkContext {
            from_location,
            to_location,
            environment: Some(environment),
            ..Default::default()
        },
    };
    let round = knowledge.begin(project, author.clone()).await.unwrap();
    let commands = [
        put(None, None, env, "legacy"),
        put(
            Some(FieldLocation::ResponseBody { status: 200 }),
            Some(FieldLocation::Path),
            env,
            "path",
        ),
        put(
            Some(FieldLocation::ResponseBody { status: 200 }),
            Some(FieldLocation::Query),
            env,
            "query",
        ),
        put(
            Some(FieldLocation::ResponseBody { status: 400 }),
            Some(FieldLocation::Path),
            env,
            "error response",
        ),
        put(
            Some(FieldLocation::ResponseBody { status: 200 }),
            Some(FieldLocation::Path),
            EnvironmentId::new(),
            "other environment",
        ),
    ];
    knowledge.apply(round, &commands).await.unwrap();
    knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let before = knowledge.links(project).await.unwrap();
    assert_eq!(before.len(), 5);
    let query = before.iter().find(|l| l.evidence == "query").unwrap();
    let legacy = before.iter().find(|l| l.evidence == "legacy").unwrap();
    let update = knowledge.begin(project, author).await.unwrap();
    knowledge
        .apply(
            update,
            &[
                put(
                    Some(FieldLocation::ResponseBody { status: 200 }),
                    Some(FieldLocation::Query),
                    env,
                    "refreshed query",
                ),
                Command::RemoveLink { id: legacy.id },
            ],
        )
        .await
        .unwrap();
    knowledge
        .finish(update, RoundState::Done, None)
        .await
        .unwrap();
    let updated = knowledge.links(project).await.unwrap();
    assert_eq!(updated.len(), 4);
    assert_eq!(
        updated
            .iter()
            .find(|l| l.evidence == "refreshed query")
            .unwrap()
            .id,
        query.id
    );
    assert!(updated.iter().any(|l| l.evidence == "path"));
    knowledge.roll_back(update).await.unwrap();
    let restored = knowledge.links(project).await.unwrap();
    assert_eq!(restored.len(), 5);
    for link in &before {
        assert!(restored.contains(link));
    }
}

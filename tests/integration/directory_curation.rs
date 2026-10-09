//! Actual PostgreSQL folder/placement/history operations with fixed model proposals.
#[path = "../../modules/curate/core/tests/directories/support.rs"]
#[allow(dead_code)]
mod support;
use async_trait::async_trait;
use nexofolio_common::{EndpointId, FolderId, ProjectId, Secret, UserId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::Organize;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_knowledge_contracts::{FolderRow, KnowledgeStore, PlacementRow};
use serde_json::{Value, json};
use std::time::Duration;
#[path = "directory_curation/protection.rs"]
mod protection;

#[derive(Clone)]
struct Facts(Vec<EndpointSummary>);
#[async_trait]
impl EndpointReader for Facts {
    async fn list(&self, _: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(self.0.clone())
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok(self.0.iter().find(|s| s.id == id).map(|s| EndpointFacts {
            requests: vec![],
            summary: s.clone(),
            fields: vec![],
            addresses: vec![],
            aliases: vec![],
            examples: vec![],
        }))
    }
    async fn example(&self, _: EndpointId, _: &str) -> Result<Option<Value>, EndpointError> {
        panic!("directory planning must not read raw samples")
    }
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn two_levels_keep_links_repeat_without_writes_move_safely_and_restore_history() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let store = PostgresKnowledge::new(&url, 4, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    let summaries = support::summaries(4);
    let project = summaries[0].project_id;
    let knowledge = Knowledge::new(store, Facts(summaries.clone()));
    let a = FolderId::new();
    let b = FolderId::new();
    let seed = knowledge.begin(project, support::author()).await.unwrap();
    let mut commands = vec![];
    for (id, name, position) in [(a, "业务甲", 0), (b, "业务乙", 1)] {
        commands.push(Command::PutFolder {
            id,
            parent: None,
            name: name.into(),
            summary: Some(format!("{name}用途")),
            position,
        });
    }
    for (index, folder) in [(0, a), (1, a), (2, b), (3, b)] {
        commands.push(Command::Place {
            endpoint: summaries[index].id,
            folder,
        });
    }
    knowledge.apply(seed, &commands).await.unwrap();
    knowledge
        .finish(seed, RoundState::Done, None)
        .await
        .unwrap();
    let before = knowledge.catalogue(project).await.unwrap();
    let old_places = snapshot(&knowledge, &summaries).await;
    let split = json!({"folders":[
        {"existing_id":a,"name":"业务甲","summary":"业务甲用途","children":[
            {"name":"查询","summary":"查询用途","endpoints":["0"]},{"name":"维护","summary":"维护用途","endpoints":["1"]}]},
        {"existing_id":b,"name":"业务乙","summary":"业务乙用途","children":[
            {"name":"查询","summary":"查询用途","endpoints":["2"]},{"name":"维护","summary":"维护用途","endpoints":["3"]}]}],"unplaced":[]});
    let pass = Organize::new(support::Canned(split.clone()), "fixed".into());
    let changes = pass
        .project_with_snapshot(&summaries, &[], &before.folders, &old_places)
        .await
        .unwrap();
    let round = knowledge.begin(project, pass.author()).await.unwrap();
    knowledge.apply(round, &changes).await.unwrap();
    knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let grown = knowledge.catalogue(project).await.unwrap();
    assert_eq!(grown.folders.len(), 6);
    assert_eq!(grown.top_level().count(), 2);
    assert_eq!(grown.unplaced, 0);
    assert_eq!(grown.children_of(Some(a)).count(), 2);
    assert_eq!(grown.children_of(Some(b)).count(), 2);
    let query_a = grown
        .folders
        .iter()
        .find(|f| f.parent == Some(a) && f.name == "查询")
        .unwrap()
        .id;
    let query_b = grown
        .folders
        .iter()
        .find(|f| f.parent == Some(b) && f.name == "查询")
        .unwrap()
        .id;
    let edit_a = grown
        .folders
        .iter()
        .find(|f| f.parent == Some(a) && f.name == "维护")
        .unwrap()
        .id;
    let edit_b = grown
        .folders
        .iter()
        .find(|f| f.parent == Some(b) && f.name == "维护")
        .unwrap()
        .id;
    assert!(
        grown.top_level().all(|f| f.endpoints == 0
            && f.endpoints_deep == 2
            && f.author == Some(support::author()))
    );
    let seats = snapshot(&knowledge, &summaries).await;
    let endpoint = knowledge.endpoint(summaries[0].id).await.unwrap();
    assert_eq!(
        endpoint.path.iter().map(|f| f.id).collect::<Vec<_>>(),
        vec![a, query_a]
    );
    let repeated = pass
        .project_with_snapshot(&summaries, &[], &grown.folders, &seats)
        .await
        .unwrap();
    assert!(repeated.is_empty());
    let rename_spec = json!({"folders":[
        {"existing_id":a,"name":"甲业务中心","summary":"业务甲用途","children":[
            {"existing_id":query_a,"name":"接口查询","summary":"查询用途","endpoints":["0"]},{"existing_id":edit_a,"name":"维护","summary":"维护用途","endpoints":["1"]}]},
        {"existing_id":b,"name":"业务乙","summary":"业务乙用途","children":[
            {"existing_id":query_b,"name":"查询","summary":"查询用途","endpoints":["2"]},{"existing_id":edit_b,"name":"维护","summary":"维护用途","endpoints":["3"]}]}],"unplaced":[]});
    let rename = Organize::new(support::Canned(rename_spec), "fixed".into());
    let changes = rename
        .project_with_snapshot(&summaries, &[], &grown.folders, &seats)
        .await
        .unwrap();
    let rename_round = knowledge.begin(project, rename.author()).await.unwrap();
    knowledge.apply(rename_round, &changes).await.unwrap();
    knowledge
        .finish(rename_round, RoundState::Done, None)
        .await
        .unwrap();
    let renamed = knowledge.catalogue(project).await.unwrap();
    assert!(
        renamed
            .folders
            .iter()
            .any(|f| f.id == a && f.name == "甲业务中心")
    );
    assert!(
        renamed
            .folders
            .iter()
            .any(|f| f.id == query_a && f.name == "接口查询")
    );
    assert_eq!(snapshot(&knowledge, &summaries).await, seats);
    let moved = json!({"folders":[
        {"existing_id":a,"name":"业务甲","summary":"业务甲用途","children":[
            {"existing_id":query_b,"name":"查询","summary":"查询用途","endpoints":["2"]},{"existing_id":edit_a,"name":"维护","summary":"维护用途","endpoints":["1"]}]},
        {"existing_id":b,"name":"业务乙","summary":"业务乙用途","children":[
            {"existing_id":query_a,"name":"查询","summary":"查询用途","endpoints":["0"]},{"existing_id":edit_b,"name":"维护","summary":"维护用途","endpoints":["3"]}]}],"unplaced":[]});
    let moving = Organize::new(support::Canned(moved), "fixed".into());
    let changes = moving
        .project_with_snapshot(&summaries, &[], &renamed.folders, &seats)
        .await
        .unwrap();
    let movement = knowledge.begin(project, moving.author()).await.unwrap();
    knowledge.apply(movement, &changes).await.unwrap();
    knowledge
        .finish(movement, RoundState::Done, None)
        .await
        .unwrap();
    let moved_catalogue = knowledge.catalogue(project).await.unwrap();
    assert!(
        moved_catalogue
            .folders
            .iter()
            .all(|f| !f.name.starts_with("__curation_"))
    );
    assert_eq!(
        knowledge
            .endpoint(summaries[0].id)
            .await
            .unwrap()
            .path
            .iter()
            .map(|f| f.id)
            .collect::<Vec<_>>(),
        vec![b, query_a]
    );
    assert_eq!(
        knowledge
            .folder_endpoints(project, Some(query_a), 1, 10)
            .await
            .unwrap()
            .items,
        vec![summaries[0].id]
    );
    knowledge.roll_back(movement).await.unwrap();
    assert_eq!(knowledge.catalogue(project).await.unwrap(), renamed);
    knowledge.roll_back(rename_round).await.unwrap();
    assert_eq!(knowledge.catalogue(project).await.unwrap(), grown);
    let grown_after_restore = knowledge.catalogue(project).await.unwrap();
    let notes = json!({"scope":"isolated PostgreSQL + constructed metadata + fixed model, no live project rewrite",
        "before":before,"after":grown_after_restore,"source_endpoint_ids":summaries.iter().map(|s| s.id).collect::<Vec<_>>(),
        "repeat_commands":repeated.len(),"rename_preserved_folder_id":a,"renamed":renamed,
        "movement_preserved_folder_id":query_a,"move_rollback":true});
    if let Ok(path) = std::env::var("DIRECTORY_EVIDENCE_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&notes).unwrap()).unwrap();
    }
    // Undo the initial split as well: legacy one-level links and placements return.
    knowledge.roll_back(round).await.unwrap();
    assert_eq!(knowledge.catalogue(project).await.unwrap(), before);
    assert_eq!(snapshot(&knowledge, &summaries).await, old_places);
    // Manual folder author is retained through refresh/history restore.
    let person = Author::Person {
        user: UserId::new(),
    };
    let manual = knowledge.begin(project, person.clone()).await.unwrap();
    knowledge
        .apply(
            manual,
            &[Command::PutFolder {
                id: a,
                parent: None,
                name: "人工业务甲".into(),
                summary: None,
                position: 0,
            }],
        )
        .await
        .unwrap();
    knowledge
        .finish(manual, RoundState::Done, None)
        .await
        .unwrap();
    let manual_tree = knowledge.catalogue(project).await.unwrap();
    assert_eq!(manual_tree.folders[0].author, Some(person));
    knowledge.roll_back(manual).await.unwrap();
    assert_eq!(knowledge.catalogue(project).await.unwrap(), before);
}

async fn snapshot<S: nexofolio_knowledge_contracts::KnowledgeStore>(
    knowledge: &Knowledge<S, Facts>,
    summaries: &[EndpointSummary],
) -> Vec<Placement> {
    let mut out = vec![];
    for endpoint in summaries {
        if let Some(p) = knowledge.endpoint(endpoint.id).await.unwrap().placement {
            out.push(p);
        }
    }
    out
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn legacy_generated_three_levels_can_be_flattened_without_deleting_shared_ids() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let store = PostgresKnowledge::new(&url, 4, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    let summaries = support::summaries(1);
    let project = summaries[0].project_id;
    let root = FolderId::new();
    let intermediate = FolderId::new();
    let leaf = FolderId::new();
    // Seed the old allowed depth through storage, as an existing database would
    // contain it. New core writes intentionally cannot create this shape.
    let mut tx = store.begin().await.unwrap();
    for (id, parent, name) in [
        (root, None, "模块"),
        (intermediate, Some(root), "旧容器"),
        (leaf, Some(intermediate), "功能"),
    ] {
        tx.put_folder(&FolderRow {
            id,
            project_id: project,
            parent,
            name: name.into(),
            summary: Some(format!("{name}用途")),
            position: 0,
            author: Some(support::author()),
        })
        .await
        .unwrap();
    }
    tx.put_placement(&PlacementRow {
        endpoint: summaries[0].id,
        project_id: project,
        folder: leaf,
        author: support::author(),
        at: "2026-10-07T00:00:00Z".parse().unwrap(),
    })
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let knowledge = Knowledge::new(store, Facts(summaries.clone()));
    let old = knowledge.catalogue(project).await.unwrap();
    let seats = snapshot(&knowledge, &summaries).await;
    let proposal = json!({"folders":[{"existing_id":root,"name":"模块","summary":"模块用途","children":[
        {"existing_id":leaf,"name":"功能","summary":"功能用途","endpoints":["0"]}]}],"unplaced":[]});
    let pass = Organize::new(support::Canned(proposal), "fixed".into());
    let changes = pass
        .project_with_snapshot(&summaries, &[], &old.folders, &seats)
        .await
        .unwrap();
    let round = knowledge.begin(project, pass.author()).await.unwrap();
    knowledge.apply(round, &changes).await.unwrap();
    knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let after = knowledge.catalogue(project).await.unwrap();
    assert_eq!(
        after.folders.len(),
        3,
        "omitted old container remains addressable"
    );
    assert!(
        after
            .folders
            .iter()
            .filter(|f| f.parent.is_some())
            .all(|f| f.parent == Some(root))
    );
    assert_eq!(
        knowledge
            .endpoint(summaries[0].id)
            .await
            .unwrap()
            .path
            .iter()
            .map(|f| f.id)
            .collect::<Vec<_>>(),
        vec![root, leaf]
    );
    knowledge.roll_back(round).await.unwrap();
    assert_eq!(knowledge.catalogue(project).await.unwrap(), old);
}

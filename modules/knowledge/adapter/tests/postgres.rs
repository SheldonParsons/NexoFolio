//! Exercises every statement the knowledge store issues against a real
//! PostgreSQL, because the SQL here is runtime strings: a wrong column name or
//! a misnumbered placeholder compiles fine and only fails when it runs.
//!
//! Needs `TEST_DATABASE_URL`; ignored by default.

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use nexofolio_common::{EndpointId, FolderId, LinkId, ProjectId, RoundId, Secret};
use nexofolio_contracts::knowledge::{
    Author, EndpointNote, FieldNote, Link, Relation, Revision, Round, RoundState, Target,
};
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_knowledge_contracts::{
    FolderRow, KnowledgeStore, KnowledgeTx, LinkIdentity, PlacementRow,
};
use serde_json::json;

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 29, 12, 0, 0).unwrap()
}

fn curate() -> Author {
    Author::Curate {
        pass: "describe".into(),
        model: None,
    }
}

async fn store() -> Option<PostgresKnowledge> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let store = PostgresKnowledge::new(&Secret::new(url), 4, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    Some(store)
}

/// A started round, so writes have something to belong to.
async fn round(tx: &mut dyn KnowledgeTx, project: ProjectId) -> RoundId {
    let id = RoundId::new();
    tx.create_round(&Round {
        id,
        project_id: project,
        author: curate(),
        started_at: at(),
        finished_at: None,
        state: RoundState::Running,
        summary: None,
    })
    .await
    .unwrap();
    id
}

#[tokio::test]
#[ignore = "needs TEST_DATABASE_URL"]
async fn every_statement_runs() {
    let Some(store) = store().await else { return };
    let project = ProjectId::new();
    let other = ProjectId::new();
    let endpoint = EndpointId::new();
    let second = EndpointId::new();

    let mut tx = store.begin().await.unwrap();
    tx.lock_project(project).await.unwrap();
    let id = round(&mut *tx, project).await;

    // -- rounds --
    let found = tx.round(id).await.unwrap().expect("round reads back");
    assert_eq!(found.state, RoundState::Running);
    assert_eq!(found.author, curate());
    assert_eq!(found.started_at, at());
    assert!(tx.rounds(other, 10).await.unwrap().is_empty());

    // -- folders --
    let top = FolderId::new();
    let child = FolderId::new();
    for (folder, parent, name) in [(top, None, "订单"), (child, Some(top), "查询")] {
        tx.put_folder(&FolderRow {
            id: folder,
            project_id: project,
            parent,
            name: name.into(),
            summary: Some("摘要".into()),
            position: 0,
        })
        .await
        .unwrap();
    }
    // Same id again replaces rather than duplicating.
    tx.put_folder(&FolderRow {
        id: top,
        project_id: project,
        parent: None,
        name: "订单管理".into(),
        summary: None,
        position: 3,
    })
    .await
    .unwrap();
    let folders = tx.folders(project).await.unwrap();
    assert_eq!(folders.len(), 2);
    let read = tx.folder(top).await.unwrap().unwrap();
    assert_eq!(read.name, "订单管理");
    assert_eq!(read.position, 3);
    assert_eq!(read.summary, None);
    assert_eq!(tx.folder(child).await.unwrap().unwrap().parent, Some(top));

    // -- placements --
    tx.put_placement(&PlacementRow {
        endpoint,
        project_id: project,
        folder: child,
        author: curate(),
        at: at(),
    })
    .await
    .unwrap();
    // One row per endpoint: placing it again moves it.
    tx.put_placement(&PlacementRow {
        endpoint,
        project_id: project,
        folder: top,
        author: curate(),
        at: at(),
    })
    .await
    .unwrap();
    assert_eq!(tx.placements(project).await.unwrap().len(), 1);
    assert_eq!(tx.placement(endpoint).await.unwrap().unwrap().folder, top);

    // -- notes --
    tx.put_endpoint_note(
        project,
        &EndpointNote {
            endpoint,
            name: Some("订单分页".into()),
            purpose: None,
            author: curate(),
            at: at(),
        },
    )
    .await
    .unwrap();
    let note = tx.endpoint_note(endpoint).await.unwrap().unwrap();
    assert_eq!(note.name.as_deref(), Some("订单分页"));
    assert_eq!(note.purpose, None);
    assert_eq!(tx.endpoint_notes(project).await.unwrap().len(), 1);
    assert!(tx.endpoint_notes(other).await.unwrap().is_empty());

    for path in ["data.list[].status", "data.total"] {
        tx.put_field_note(
            project,
            &FieldNote {
                endpoint,
                path: path.into(),
                text: "状态".into(),
                values: vec!["1".into(), "2".into()],
                author: curate(),
                at: at(),
            },
        )
        .await
        .unwrap();
    }
    let notes = tx.field_notes(endpoint).await.unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].values, vec!["1".to_string(), "2".to_string()]);
    tx.remove_field_note(endpoint, "data.total").await.unwrap();
    assert_eq!(tx.field_notes(endpoint).await.unwrap().len(), 1);

    // -- links --
    let link = Link {
        id: LinkId::new(),
        from: endpoint,
        from_field: Some("data.list[].seriesId".into()),
        to: second,
        to_field: None,
        relation: Relation::Dictionary,
        evidence: "12 个值全部命中".into(),
        author: curate(),
        at: at(),
    };
    tx.put_link(project, &link).await.unwrap();
    // Rediscovery refreshes the same row: identity is the ends plus relation.
    let again = Link {
        id: LinkId::new(),
        evidence: "18 个值全部命中".into(),
        ..link.clone()
    };
    tx.put_link(project, &again).await.unwrap();
    let links = tx.links(project).await.unwrap();
    assert_eq!(links.len(), 1, "identity conflict must update, not insert");
    assert_eq!(links[0].id, link.id, "the first id survives");
    assert_eq!(links[0].evidence, "18 个值全部命中");
    assert_eq!(links[0].from_field.as_deref(), Some("data.list[].seriesId"));

    let identity = LinkIdentity {
        from: link.from,
        from_field: link.from_field.clone(),
        to: link.to,
        to_field: None,
        relation: Relation::Dictionary,
    };
    assert_eq!(tx.find_link(&identity).await.unwrap().unwrap().id, link.id);
    // A NULL field must not match a named one.
    assert!(
        tx.find_link(&LinkIdentity {
            from_field: None,
            ..identity.clone()
        })
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(tx.links_of(second).await.unwrap().len(), 1, "either end");
    assert_eq!(tx.link(link.id).await.unwrap().unwrap().to, second);

    // -- revisions come back newest first --
    for seq in 1..=3 {
        tx.record_revision(&Revision {
            round: id,
            seq,
            target: Target::Folder { id: top },
            before: (seq > 1).then(|| json!({"seq": seq})),
        })
        .await
        .unwrap();
    }
    let revisions = tx.revisions(id).await.unwrap();
    assert_eq!(
        revisions.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    assert_eq!(revisions[2].before, None);
    assert_eq!(revisions[0].target, Target::Folder { id: top });

    tx.finish_round(id, RoundState::Done, Some("完成"), at())
        .await
        .unwrap();
    let done = tx.round(id).await.unwrap().unwrap();
    assert_eq!(done.state, RoundState::Done);
    assert_eq!(done.finished_at, Some(at()));
    assert_eq!(done.summary.as_deref(), Some("完成"));

    // Removing a folder frees what sat in it.
    tx.remove_folder(top).await.unwrap();
    assert!(tx.folders(project).await.unwrap().is_empty(), "cascade");
    assert!(tx.placement(endpoint).await.unwrap().is_none());
    tx.remove_link(link.id).await.unwrap();
    assert!(tx.links(project).await.unwrap().is_empty());

    tx.commit().await.unwrap();
    store.close().await;
}

/// `restore` has to be the exact reverse of each write, for both directions:
/// `before: None` removes, `before: Some(_)` puts the old value back.
#[tokio::test]
#[ignore = "needs TEST_DATABASE_URL"]
async fn restore_reverses_every_write() {
    let Some(store) = store().await else { return };
    let project = ProjectId::new();
    let endpoint = EndpointId::new();
    let second = EndpointId::new();

    let mut tx = store.begin().await.unwrap();
    let _ = round(&mut *tx, project).await;

    let folder = FolderId::new();
    let kept = FolderRow {
        id: folder,
        project_id: project,
        parent: None,
        name: "原名".into(),
        summary: Some("原摘要".into()),
        position: 2,
    };
    tx.put_folder(&kept).await.unwrap();
    tx.put_folder(&FolderRow {
        name: "改名".into(),
        summary: None,
        position: 7,
        ..kept.clone()
    })
    .await
    .unwrap();
    tx.restore(
        project,
        &Target::Folder { id: folder },
        Some(&json!({
            "parent": kept.parent, "name": kept.name,
            "summary": kept.summary, "position": kept.position,
        })),
    )
    .await
    .unwrap();
    assert_eq!(tx.folder(folder).await.unwrap().unwrap(), kept);

    tx.put_placement(&PlacementRow {
        endpoint,
        project_id: project,
        folder,
        author: curate(),
        at: at(),
    })
    .await
    .unwrap();
    tx.restore(project, &Target::Placement { endpoint }, None)
        .await
        .unwrap();
    assert!(tx.placement(endpoint).await.unwrap().is_none());
    // And back again, from the values a revision keeps.
    tx.restore(
        project,
        &Target::Placement { endpoint },
        Some(&json!({ "folder": folder, "author": curate(), "at": at() })),
    )
    .await
    .unwrap();
    let back = tx.placement(endpoint).await.unwrap().unwrap();
    assert_eq!(
        (back.folder, back.author, back.at),
        (folder, curate(), at())
    );

    tx.restore(
        project,
        &Target::EndpointNote { endpoint },
        Some(&json!({
            "name": "旧名", "purpose": null, "author": curate(), "at": at(),
        })),
    )
    .await
    .unwrap();
    assert_eq!(
        tx.endpoint_note(endpoint).await.unwrap().unwrap().name,
        Some("旧名".into())
    );
    tx.restore(project, &Target::EndpointNote { endpoint }, None)
        .await
        .unwrap();
    assert!(tx.endpoint_note(endpoint).await.unwrap().is_none());

    let path = "data.list[].status".to_string();
    tx.restore(
        project,
        &Target::FieldNote {
            endpoint,
            path: path.clone(),
        },
        Some(&json!({
            "text": "旧说明", "values": ["1"], "author": curate(), "at": at(),
        })),
    )
    .await
    .unwrap();
    let note = tx.field_note(endpoint, &path).await.unwrap().unwrap();
    assert_eq!((note.text.as_str(), note.values.len()), ("旧说明", 1));
    tx.restore(
        project,
        &Target::FieldNote {
            endpoint,
            path: path.clone(),
        },
        None,
    )
    .await
    .unwrap();
    assert!(tx.field_note(endpoint, &path).await.unwrap().is_none());

    let link = LinkId::new();
    tx.restore(
        project,
        &Target::Link { id: link },
        Some(&json!({
            "from": endpoint, "from_field": "a.b", "to": second, "to_field": null,
            "relation": "dictionary", "evidence": "旧证据",
            "author": curate(), "at": at(),
        })),
    )
    .await
    .unwrap();
    let back = tx.link(link).await.unwrap().unwrap();
    assert_eq!(back.relation, Relation::Dictionary);
    assert_eq!(back.evidence, "旧证据");
    assert_eq!(back.from_field.as_deref(), Some("a.b"));
    tx.restore(project, &Target::Link { id: link }, None)
        .await
        .unwrap();
    assert!(tx.link(link).await.unwrap().is_none());

    tx.commit().await.unwrap();
    store.close().await;
}

/// Dropping a transaction without committing must leave nothing behind.
#[tokio::test]
#[ignore = "needs TEST_DATABASE_URL"]
async fn uncommitted_writes_vanish() {
    let Some(store) = store().await else { return };
    let project = ProjectId::new();

    let mut tx = store.begin().await.unwrap();
    let id = round(&mut *tx, project).await;
    drop(tx);

    let mut tx = store.begin().await.unwrap();
    assert!(tx.round(id).await.unwrap().is_none());
    assert!(tx.rounds(project, 10).await.unwrap().is_empty());
    tx.commit().await.unwrap();
    store.close().await;
}

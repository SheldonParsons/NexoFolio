//! What knowledge promises, over the in-memory store: the tree rules it keeps
//! out of SQL, 待分类 as a subtraction, rollback, and the guards around rounds.

use chrono::{DateTime, TimeZone, Utc};
use nexofolio_common::{EndpointId, EnvironmentId, FolderId, ProjectId};
use nexofolio_contracts::endpoint::{EndpointFacts, EndpointSummary, EnvironmentUsage};
use nexofolio_contracts::knowledge::{
    Author, Command, KnowledgeError, KnowledgeReader, KnowledgeWriter, Relation, RoundState,
};
use nexofolio_contracts::testing::InMemoryEndpoints;
use nexofolio_knowledge::{Clock, Knowledge};
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;

struct Fixed;

impl Clock for Fixed {
    fn now(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 29, 12, 0, 0).unwrap()
    }
}

fn curate() -> Author {
    Author::Curate {
        pass: "catalogue".into(),
        model: None,
    }
}

/// Observe seeded with `count` endpoints of one project.
fn observed(project: ProjectId, count: usize) -> (InMemoryEndpoints, Vec<EndpointId>) {
    let endpoints = InMemoryEndpoints::new();
    let mut ids = Vec::new();
    for n in 0..count {
        let id = EndpointId::new();
        ids.push(id);
        endpoints.put(EndpointFacts {
            summary: EndpointSummary {
                id,
                project_id: project,
                method: "GET".into(),
                path_template: format!("/api/thing/{n}"),
                external: false,
                declared: false,
                environments: vec![EnvironmentUsage {
                    environment_id: EnvironmentId::new(),
                    calls: 1,
                    first_seen: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
                    last_seen: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
                }],
            },
            aliases: Vec::new(),
            addresses: Vec::new(),
            fields: Vec::new(),
            examples: Vec::new(),
        });
    }
    (endpoints, ids)
}

fn fixture(
    project: ProjectId,
    count: usize,
) -> (
    Knowledge<InMemoryKnowledge, InMemoryEndpoints, Fixed>,
    Vec<EndpointId>,
) {
    let (endpoints, ids) = observed(project, count);
    (
        Knowledge::with_clock(InMemoryKnowledge::new(), endpoints, Fixed),
        ids,
    )
}

/// An endpoint observe accepts is browsable at once, with nothing written here.
#[tokio::test]
async fn unplaced_endpoints_need_no_write() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 47);

    let catalogue = knowledge.catalogue(project).await.unwrap();
    assert_eq!(catalogue.unplaced, 47);
    assert!(catalogue.folders.is_empty());
    let unplaced = knowledge
        .folder_endpoints(project, None, 1, 100)
        .await
        .unwrap();
    assert_eq!(unplaced.total, 47);
    assert_eq!(unplaced.items.len(), 47);

    let round = knowledge.begin(project, curate()).await.unwrap();
    let folder = FolderId::new();
    knowledge
        .apply(
            round,
            &[
                Command::PutFolder {
                    id: folder,
                    parent: None,
                    name: "订单".into(),
                    summary: None,
                    position: 0,
                },
                Command::Place {
                    endpoint: ids[0],
                    folder,
                },
            ],
        )
        .await
        .unwrap();

    let catalogue = knowledge.catalogue(project).await.unwrap();
    assert_eq!(catalogue.unplaced, 46, "placing one leaves 46 unplaced");
    assert_eq!(catalogue.folders.len(), 1);
    assert_eq!(catalogue.folders[0].endpoints, 1);
    assert_eq!(
        knowledge
            .folder_endpoints(project, Some(folder), 1, 100)
            .await
            .unwrap()
            .items,
        vec![ids[0]]
    );
}

/// Counts roll up: a parent reports its own placements and its subtree's.
#[tokio::test]
async fn deep_counts_include_the_subtree() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 3);
    let round = knowledge.begin(project, curate()).await.unwrap();

    let top = FolderId::new();
    let child = FolderId::new();
    knowledge
        .apply(
            round,
            &[
                Command::PutFolder {
                    id: top,
                    parent: None,
                    name: "订单".into(),
                    summary: None,
                    position: 0,
                },
                Command::PutFolder {
                    id: child,
                    parent: Some(top),
                    name: "查询".into(),
                    summary: None,
                    position: 0,
                },
                Command::Place {
                    endpoint: ids[0],
                    folder: top,
                },
                Command::Place {
                    endpoint: ids[1],
                    folder: child,
                },
            ],
        )
        .await
        .unwrap();

    let catalogue = knowledge.catalogue(project).await.unwrap();
    let parent = catalogue.folders.iter().find(|f| f.id == top).unwrap();
    assert_eq!(parent.endpoints, 1, "its own placements only");
    assert_eq!(parent.endpoints_deep, 2, "and the subtree's");
    assert_eq!(catalogue.unplaced, 1);
}

/// Placing an endpoint again moves it: the tree is a real tree.
#[tokio::test]
async fn an_endpoint_sits_in_one_folder() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 1);
    let round = knowledge.begin(project, curate()).await.unwrap();

    let first = FolderId::new();
    let second = FolderId::new();
    for (id, name) in [(first, "订单"), (second, "用户")] {
        knowledge
            .apply(
                round,
                &[Command::PutFolder {
                    id,
                    parent: None,
                    name: name.into(),
                    summary: None,
                    position: 0,
                }],
            )
            .await
            .unwrap();
    }
    for folder in [first, second] {
        knowledge
            .apply(
                round,
                &[Command::Place {
                    endpoint: ids[0],
                    folder,
                }],
            )
            .await
            .unwrap();
    }

    let catalogue = knowledge.catalogue(project).await.unwrap();
    assert_eq!(catalogue.unplaced, 0);
    let seats: usize = catalogue.folders.iter().map(|f| f.endpoints).sum();
    assert_eq!(seats, 1, "moved, not duplicated");
    let known = knowledge.endpoint(ids[0]).await.unwrap();
    assert_eq!(known.placement.unwrap().folder, second);
}

/// Depth and ancestry are checked here because both need the whole tree.
#[tokio::test]
async fn the_tree_stays_shallow_and_acyclic() {
    let project = ProjectId::new();
    let (knowledge, _) = fixture(project, 1);
    let round = knowledge.begin(project, curate()).await.unwrap();

    let mut chain = Vec::new();
    let mut parent = None;
    // Three levels are allowed.
    for level in 0..3 {
        let id = FolderId::new();
        chain.push(id);
        knowledge
            .apply(
                round,
                &[Command::PutFolder {
                    id,
                    parent,
                    name: format!("层{level}"),
                    summary: None,
                    position: 0,
                }],
            )
            .await
            .unwrap();
        parent = Some(id);
    }

    // A fourth is not.
    let too_deep = knowledge
        .apply(
            round,
            &[Command::PutFolder {
                id: FolderId::new(),
                parent,
                name: "第四层".into(),
                summary: None,
                position: 0,
            }],
        )
        .await;
    assert!(matches!(too_deep, Err(KnowledgeError::InvalidTree(_))));

    // Nor is a folder becoming its own ancestor.
    let cycle = knowledge
        .apply(
            round,
            &[Command::PutFolder {
                id: chain[0],
                parent: Some(chain[2]),
                name: "层0".into(),
                summary: None,
                position: 0,
            }],
        )
        .await;
    assert!(matches!(cycle, Err(KnowledgeError::InvalidTree(_))));

    let itself = knowledge
        .apply(
            round,
            &[Command::PutFolder {
                id: chain[0],
                parent: Some(chain[0]),
                name: "层0".into(),
                summary: None,
                position: 0,
            }],
        )
        .await;
    assert!(matches!(itself, Err(KnowledgeError::InvalidTree(_))));

    // Placing into a folder that does not exist is rejected too.
    let nowhere = knowledge
        .apply(
            round,
            &[Command::Place {
                endpoint: EndpointId::new(),
                folder: FolderId::new(),
            }],
        )
        .await;
    assert!(matches!(
        nowhere,
        Err(KnowledgeError::WrongProject | KnowledgeError::NoSuchFolder)
    ));
    assert_eq!(knowledge.catalogue(project).await.unwrap().folders.len(), 3);
}

/// A name written without a purpose must not erase a purpose someone wrote.
#[tokio::test]
async fn writing_one_note_field_keeps_the_other() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 1);
    let round = knowledge.begin(project, curate()).await.unwrap();

    knowledge
        .apply(
            round,
            &[Command::PutEndpointNote {
                endpoint: ids[0],
                name: Some("订单分页".into()),
                purpose: Some("手写的用途".into()),
            }],
        )
        .await
        .unwrap();
    knowledge
        .apply(
            round,
            &[Command::PutEndpointNote {
                endpoint: ids[0],
                name: Some("订单分页查询".into()),
                purpose: None,
            }],
        )
        .await
        .unwrap();

    let note = knowledge.endpoint(ids[0]).await.unwrap().note.unwrap();
    assert_eq!(note.name.as_deref(), Some("订单分页查询"));
    assert_eq!(note.purpose.as_deref(), Some("手写的用途"));
}

/// Rediscovering a link refreshes its evidence instead of adding a duplicate.
#[tokio::test]
async fn links_have_one_row_per_identity() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 2);
    let round = knowledge.begin(project, curate()).await.unwrap();

    for evidence in ["12 个值命中", "18 个值命中"] {
        knowledge
            .apply(
                round,
                &[Command::PutLink {
                    from: ids[0],
                    from_field: Some("data.list[].seriesId".into()),
                    to: ids[1],
                    to_field: None,
                    relation: Relation::Dictionary,
                    evidence: evidence.into(),
                }],
            )
            .await
            .unwrap();
    }

    let links = knowledge.links(project).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].evidence, "18 个值命中");
    // Both ends can find it.
    assert_eq!(knowledge.endpoint(ids[1]).await.unwrap().links.len(), 1);
}

/// Undoing a round puts back exactly what it replaced, in reverse order.
#[tokio::test]
async fn rolling_back_restores_what_was_there() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 2);

    // A first round establishes a state worth getting back to.
    let first = knowledge.begin(project, curate()).await.unwrap();
    let folder = FolderId::new();
    knowledge
        .apply(
            first,
            &[
                Command::PutFolder {
                    id: folder,
                    parent: None,
                    name: "订单".into(),
                    summary: Some("原摘要".into()),
                    position: 1,
                },
                Command::Place {
                    endpoint: ids[0],
                    folder,
                },
                Command::PutEndpointNote {
                    endpoint: ids[0],
                    name: Some("原名".into()),
                    purpose: None,
                },
            ],
        )
        .await
        .unwrap();
    knowledge
        .finish(first, RoundState::Done, None)
        .await
        .unwrap();

    // A second round changes all of it and adds something new.
    let second = knowledge.begin(project, curate()).await.unwrap();
    knowledge
        .apply(
            second,
            &[
                Command::PutFolder {
                    id: folder,
                    parent: None,
                    name: "订单管理".into(),
                    summary: None,
                    position: 9,
                },
                Command::PutEndpointNote {
                    endpoint: ids[0],
                    name: Some("改名".into()),
                    purpose: None,
                },
                Command::Place {
                    endpoint: ids[1],
                    folder,
                },
                Command::PutLink {
                    from: ids[0],
                    from_field: None,
                    to: ids[1],
                    to_field: None,
                    relation: Relation::ListToDetail,
                    evidence: "id 对得上".into(),
                },
            ],
        )
        .await
        .unwrap();
    knowledge
        .finish(second, RoundState::Done, None)
        .await
        .unwrap();

    knowledge.roll_back(second).await.unwrap();

    let catalogue = knowledge.catalogue(project).await.unwrap();
    let back = &catalogue.folders[0];
    assert_eq!(
        (back.name.as_str(), back.summary.as_deref(), back.position),
        ("订单", Some("原摘要"), 1),
        "the first round's values come back"
    );
    let known = knowledge.endpoint(ids[0]).await.unwrap();
    assert_eq!(known.note.unwrap().name.as_deref(), Some("原名"));
    assert!(
        knowledge.links(project).await.unwrap().is_empty(),
        "what the round created is gone"
    );
    assert_eq!(catalogue.unplaced, 1, "and so is the placement it added");

    // The first round's work survives untouched.
    assert_eq!(known.placement.unwrap().folder, folder);
    let rounds = knowledge.rounds(project, 10).await.unwrap();
    assert_eq!(rounds.len(), 2);
    assert!(
        rounds
            .iter()
            .any(|r| r.id == second && r.state == RoundState::RolledBack)
    );
}

/// A finished round is what rollback replays, so nothing may be appended to it
/// and it may not finish twice.
#[tokio::test]
async fn a_finished_round_takes_no_more_writes() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 1);

    let round = knowledge.begin(project, curate()).await.unwrap();
    knowledge
        .finish(round, RoundState::Done, Some("完成".into()))
        .await
        .unwrap();

    let late = knowledge
        .apply(
            round,
            &[Command::PutEndpointNote {
                endpoint: ids[0],
                name: Some("迟到的写入".into()),
                purpose: None,
            }],
        )
        .await;
    assert_eq!(late, Err(KnowledgeError::RoundClosed));
    assert!(knowledge.endpoint(ids[0]).await.unwrap().note.is_none());

    // Finishing twice would overwrite how it ended.
    assert_eq!(
        knowledge.finish(round, RoundState::Failed, None).await,
        Err(KnowledgeError::RoundClosed)
    );
    // And a rollback cannot be replayed.
    knowledge.roll_back(round).await.unwrap();
    assert_eq!(
        knowledge.roll_back(round).await,
        Err(KnowledgeError::RoundClosed)
    );
}

/// A round may only touch its own project's endpoints, and an unknown round is
/// not a silent no-op.
#[tokio::test]
async fn writes_stay_inside_the_project() {
    let project = ProjectId::new();
    let (knowledge, _) = fixture(project, 1);
    let round = knowledge.begin(project, curate()).await.unwrap();

    let foreign = knowledge
        .apply(
            round,
            &[Command::PutEndpointNote {
                endpoint: EndpointId::new(),
                name: Some("别的项目".into()),
                purpose: None,
            }],
        )
        .await;
    assert_eq!(foreign, Err(KnowledgeError::WrongProject));

    let unknown = knowledge.apply(nexofolio_common::RoundId::new(), &[]).await;
    assert_eq!(unknown, Err(KnowledgeError::NoSuchRound));
}

/// The detail view carries the breadcrumb, top level first.
#[tokio::test]
async fn endpoint_detail_knows_its_path() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 1);
    let round = knowledge.begin(project, curate()).await.unwrap();

    let top = FolderId::new();
    let child = FolderId::new();
    knowledge
        .apply(
            round,
            &[
                Command::PutFolder {
                    id: top,
                    parent: None,
                    name: "订单".into(),
                    summary: None,
                    position: 0,
                },
                Command::PutFolder {
                    id: child,
                    parent: Some(top),
                    name: "查询".into(),
                    summary: None,
                    position: 0,
                },
                Command::Place {
                    endpoint: ids[0],
                    folder: child,
                },
                Command::PutFieldNote {
                    endpoint: ids[0],
                    path: "data.list[].status".into(),
                    text: "订单状态".into(),
                    values: vec!["1".into(), "2".into()],
                },
            ],
        )
        .await
        .unwrap();

    let known = knowledge.endpoint(ids[0]).await.unwrap();
    assert_eq!(
        known
            .path
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["订单", "查询"]
    );
    assert_eq!(known.fields.len(), 1);
    assert_eq!(known.fields[0].values, vec!["1".to_string(), "2".into()]);

    // Nothing written about an endpoint is the normal state, not an error.
    let (bare, bare_ids) = fixture(ProjectId::new(), 1);
    let empty = bare.endpoint(bare_ids[0]).await.unwrap();
    assert!(empty.note.is_none() && empty.path.is_empty() && empty.fields.is_empty());
}

/// Removing a folder frees the endpoints in it and in its subtree, rather than
/// leaving them pointing at something gone.
#[tokio::test]
async fn removing_a_folder_frees_its_endpoints() {
    let project = ProjectId::new();
    let (knowledge, ids) = fixture(project, 2);
    let round = knowledge.begin(project, curate()).await.unwrap();

    let top = FolderId::new();
    let child = FolderId::new();
    knowledge
        .apply(
            round,
            &[
                Command::PutFolder {
                    id: top,
                    parent: None,
                    name: "订单".into(),
                    summary: None,
                    position: 0,
                },
                Command::PutFolder {
                    id: child,
                    parent: Some(top),
                    name: "查询".into(),
                    summary: None,
                    position: 0,
                },
                Command::Place {
                    endpoint: ids[0],
                    folder: top,
                },
                Command::Place {
                    endpoint: ids[1],
                    folder: child,
                },
                Command::RemoveFolder { id: top },
            ],
        )
        .await
        .unwrap();

    let catalogue = knowledge.catalogue(project).await.unwrap();
    assert!(catalogue.folders.is_empty(), "subtree goes with it");
    assert_eq!(catalogue.unplaced, 2, "and both endpoints are unplaced");

    assert_eq!(
        knowledge
            .apply(round, &[Command::RemoveFolder { id: top }])
            .await,
        Err(KnowledgeError::NoSuchFolder)
    );
}

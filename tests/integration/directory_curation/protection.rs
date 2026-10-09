//! Check protection again under the actual project transaction, after model time.
use super::*;
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn edits_after_the_model_snapshot_abort_automatic_changes_atomically() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let store = PostgresKnowledge::new(&url, 4, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    let summaries = support::summaries(1);
    let project = summaries[0].project_id;
    let knowledge = Knowledge::new(store, Facts(summaries.clone()));
    let root = FolderId::new();
    let leaf = FolderId::new();
    let seed = knowledge.begin(project, support::author()).await.unwrap();
    knowledge
        .apply(
            seed,
            &[
                Command::PutFolder {
                    id: root,
                    parent: None,
                    name: "业务".into(),
                    summary: Some("业务用途".into()),
                    position: 0,
                },
                Command::PutFolder {
                    id: leaf,
                    parent: Some(root),
                    name: "查询".into(),
                    summary: Some("查询用途".into()),
                    position: 0,
                },
                Command::Place {
                    endpoint: summaries[0].id,
                    folder: leaf,
                },
            ],
        )
        .await
        .unwrap();
    knowledge
        .finish(seed, RoundState::Done, None)
        .await
        .unwrap();
    let snapshot_folders = knowledge.catalogue(project).await.unwrap();
    let seats = snapshot(&knowledge, &summaries).await;
    let pass = Organize::new(
        support::Canned(
            json!({"folders":[{"existing_id":root,"name":"业务","summary":"业务用途","children":[
        {"existing_id":leaf,"name":"自动新名称","summary":"查询用途","endpoints":["0"]}]}],"unplaced":[]}),
        ),
        "fixed".into(),
    );
    let mut proposed = pass
        .project_with_snapshot(&summaries, &[], &snapshot_folders.folders, &seats)
        .await
        .unwrap();
    // A user edits after the planner saw generated ownership.
    let person = Author::Person {
        user: UserId::new(),
    };
    let manual = knowledge.begin(project, person.clone()).await.unwrap();
    knowledge
        .apply(
            manual,
            &[
                Command::PutFolder {
                    id: leaf,
                    parent: Some(root),
                    name: "人工确认名称".into(),
                    summary: Some("人工介绍".into()),
                    position: 0,
                },
                Command::Place {
                    endpoint: summaries[0].id,
                    folder: leaf,
                },
            ],
        )
        .await
        .unwrap();
    knowledge
        .finish(manual, RoundState::Done, None)
        .await
        .unwrap();
    let before = knowledge.catalogue(project).await.unwrap();
    let transient = FolderId::new();
    proposed.insert(
        0,
        Command::PutFolder {
            id: transient,
            parent: None,
            name: "不应提交的目录".into(),
            summary: Some("测试事务".into()),
            position: 1,
        },
    );
    let automatic = knowledge.begin(project, pass.author()).await.unwrap();
    assert!(matches!(
        knowledge.apply(automatic, &proposed).await,
        Err(KnowledgeError::ProtectedKnowledge)
    ));
    knowledge
        .finish(automatic, RoundState::Failed, None)
        .await
        .unwrap();
    assert_eq!(
        knowledge.catalogue(project).await.unwrap(),
        before,
        "earlier writes in the same batch roll back"
    );
    assert!(
        !knowledge
            .catalogue(project)
            .await
            .unwrap()
            .folders
            .iter()
            .any(|f| f.id == transient)
    );
    let automatic = knowledge.begin(project, pass.author()).await.unwrap();
    assert!(matches!(
        knowledge
            .apply(
                automatic,
                &[Command::Place {
                    endpoint: summaries[0].id,
                    folder: root
                }]
            )
            .await,
        Err(KnowledgeError::ProtectedKnowledge)
    ));
    assert!(matches!(
        knowledge
            .apply(
                automatic,
                &[Command::Unplace {
                    endpoint: summaries[0].id
                }]
            )
            .await,
        Err(KnowledgeError::ProtectedKnowledge)
    ));
    // An identical placement is a true no-op and retains the person's author.
    knowledge
        .apply(
            automatic,
            &[Command::Place {
                endpoint: summaries[0].id,
                folder: leaf,
            }],
        )
        .await
        .unwrap();
    knowledge
        .finish(automatic, RoundState::Done, None)
        .await
        .unwrap();
    let current = knowledge.endpoint(summaries[0].id).await.unwrap();
    assert_eq!(current.placement.unwrap().author, person);
    assert_eq!(current.path.last().unwrap().name, "人工确认名称");
}

//! The same naming rules run against memory and PostgreSQL.
use super::*;

pub async fn folder_names<S: KnowledgeStore + Clone>(store: S) {
    for retire_root in [true, false] {
        let facts = facts();
        let (project, endpoint, _) = facts.ids();
        let k = Knowledge::new(store.clone(), facts);
        let user = actor();
        let manual = Author::Person {
            user: user.user_id().unwrap(),
        };
        let root = FolderId::new();
        let child = FolderId::new();
        let original = k.begin(project, manual.clone()).await.unwrap();
        let folder = |id, parent, name: &str| Command::PutFolder {
            id,
            parent,
            name: name.into(),
            summary: Some("保留原始介绍".into()),
            position: 0,
        };
        k.apply(
            original,
            &[
                folder(root, None, "业务模块"),
                folder(child, Some(root), "业务功能"),
                Command::Place {
                    endpoint,
                    folder: child,
                },
            ],
        )
        .await
        .unwrap();
        k.finish(original, RoundState::Done, None).await.unwrap();
        let old_target = if retire_root { root } else { child };
        k.change_lifecycle(
            project,
            change(LifecycleTarget::Folder { id: old_target }, true, 0),
            user.clone(),
        )
        .await
        .unwrap();

        let candidate = k.create_candidate(project, None, None).await.unwrap();
        let batch = k
            .open_candidate_batch(
                project,
                candidate.id,
                None,
                Author::Curate {
                    pass: "organize".into(),
                    model: None,
                },
            )
            .await
            .unwrap();
        let new_root = if retire_root { FolderId::new() } else { root };
        let new_child = FolderId::new();
        let mut commands = Vec::new();
        if retire_root {
            commands.push(folder(new_root, None, "业务模块"));
        }
        commands.push(folder(new_child, Some(new_root), "业务功能"));
        commands.push(Command::Place {
            endpoint,
            folder: new_child,
        });
        k.append_candidate(project, candidate.id, None, batch, &commands)
            .await
            .unwrap();
        k.close_candidate_batch(project, candidate.id, None, batch, RoundState::Done)
            .await
            .unwrap();
        ready(&k, project, &candidate).await;
        k.apply_candidate(project, candidate.id, user.user_id().unwrap())
            .await
            .unwrap();
        assert_eq!(
            k.endpoint(endpoint)
                .await
                .unwrap()
                .placement
                .unwrap()
                .folder,
            new_child
        );
        let catalogue = k.catalogue(project).await.unwrap();
        assert_eq!(
            catalogue
                .folders
                .iter()
                .find(|f| f.id == root)
                .unwrap()
                .name,
            "业务模块"
        );
        assert_eq!(
            catalogue
                .folders
                .iter()
                .find(|f| f.id == child)
                .unwrap()
                .summary
                .as_deref(),
            Some("保留原始介绍")
        );

        // A conflicting undo is actionable and atomic, never an unavailable DB error.
        let undo = k
            .change_lifecycle(
                project,
                change(LifecycleTarget::Folder { id: old_target }, false, 1),
                user.clone(),
            )
            .await
            .unwrap_err();
        assert!(matches!(undo, KnowledgeError::InvalidLifecycle(ref m) if m.contains("同名目录")));
        let state = k.lifecycle(project).await.unwrap();
        assert_eq!((state.version, state.reconciled_version), (1, 1));
        assert!(state.folder(old_target).is_some());

        let replacement = if retire_root { new_root } else { new_child };
        k.change_lifecycle(
            project,
            change(LifecycleTarget::Folder { id: replacement }, true, 1),
            user.clone(),
        )
        .await
        .unwrap();
        k.change_lifecycle(
            project,
            change(LifecycleTarget::Folder { id: old_target }, false, 2),
            user.clone(),
        )
        .await
        .unwrap();
        assert!(
            k.lifecycle(project)
                .await
                .unwrap()
                .folder(old_target)
                .is_none()
        );

        let (parent, name) = if retire_root {
            (None, "业务模块")
        } else {
            (Some(root), "业务功能")
        };
        let duplicate = FolderId::new();
        let write = k.begin(project, manual.clone()).await.unwrap();
        assert!(matches!(
            k.apply(write, &[folder(duplicate, parent, name)]).await,
            Err(KnowledgeError::InvalidTree(_))
        ));
        assert!(
            !k.catalogue(project)
                .await
                .unwrap()
                .folders
                .iter()
                .any(|f| f.id == duplicate)
        );

        // Rolling back a rename must not create two effective same-name siblings.
        let rename = k.begin(project, manual.clone()).await.unwrap();
        k.apply(rename, &[folder(old_target, parent, "临时名称")])
            .await
            .unwrap();
        k.finish(rename, RoundState::Done, None).await.unwrap();
        k.apply(write, &[folder(duplicate, parent, name)])
            .await
            .unwrap();
        assert!(matches!(
            k.roll_back(rename).await,
            Err(KnowledgeError::InvalidTree(_))
        ));
        assert_eq!(
            k.catalogue(project)
                .await
                .unwrap()
                .folders
                .iter()
                .find(|f| f.id == old_target)
                .unwrap()
                .name,
            "临时名称"
        );
    }
}

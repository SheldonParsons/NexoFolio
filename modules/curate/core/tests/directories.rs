#[path = "directories/support.rs"]
mod support;
use nexofolio_common::{FolderId, UserId};
use nexofolio_contracts::knowledge::*;
use serde_json::json;
use support::*;

#[tokio::test]
async fn semantic_keys_distinguish_methods_even_when_paths_are_identical() {
    let ss = summaries(3);
    let mut spec = basic();
    spec["unplaced"] = json!([{"key":"2","reason":"资料不足"}]);
    let commands = commands(spec, &ss, &[], &[]).await.unwrap();
    let roots: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::PutFolder {
                id, parent: None, ..
            } => Some(*id),
            _ => None,
        })
        .collect();
    let leaves: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::PutFolder {
                id,
                parent: Some(p),
                ..
            } => Some((*id, *p)),
            _ => None,
        })
        .collect();
    assert_eq!(roots.len(), 1);
    assert_eq!(leaves.len(), 2);
    assert!(leaves.iter().all(|(_, p)| *p == roots[0]));
    let placed: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::Place { endpoint, folder } => Some((*endpoint, *folder)),
            _ => None,
        })
        .collect();
    assert_eq!(
        placed,
        vec![(ss[0].id, leaves[0].0), (ss[1].id, leaves[1].0)]
    );
}

#[tokio::test]
async fn invalid_shape_third_level_duplicate_missing_unknown_or_empty_groups_propose_no_writes() {
    let ss = summaries(2);
    let mut invalid = vec![json!([{"name":"账户","summary":"账户","match":["/gateway/"]}])];
    let mut p = basic();
    p["folders"][0]["children"][0]["children"] = json!([]);
    invalid.push(p);
    let mut p = basic();
    p["folders"][0]["children"][1]["endpoints"] = json!(["0", "1"]);
    invalid.push(p);
    let mut p = basic();
    p["folders"][0]["children"][1]["endpoints"] = json!([]);
    invalid.push(p);
    let mut p = basic();
    p["folders"][0]["children"][1]["endpoints"] = json!(["99"]);
    invalid.push(p);
    let mut p = basic();
    p["folders"][0]["existing_id"] = json!(FolderId::new());
    invalid.push(p);
    let mut p = basic();
    p["folders"][0]["children"][1]["name"] = json!("查询");
    invalid.push(p);
    let mut p = basic();
    p["folders"][0]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"空","summary":"空用途","endpoints":[]}));
    invalid.push(p);
    for p in invalid {
        assert!(commands(p, &ss, &[], &[]).await.is_err());
    }
}

#[tokio::test]
async fn repeated_structure_reuses_ids_and_writes_nothing_and_exact_membership_can_rename() {
    let ss = summaries(2);
    let root = FolderId::new();
    let query = FolderId::new();
    let edit = FolderId::new();
    let existing = vec![
        folder(root, None, "账户", 0),
        folder(query, Some(root), "查询", 0),
        folder(edit, Some(root), "维护", 1),
    ];
    let seats = vec![placement(ss[0].id, query), placement(ss[1].id, edit)];
    assert!(
        commands(basic(), &ss, &existing, &seats)
            .await
            .unwrap()
            .is_empty()
    );
    let mut omitted = basic();
    omitted["folders"][0]["summary"] = json!(null);
    omitted["folders"][0]["children"][0]["summary"] = json!(null);
    assert!(
        commands(omitted, &ss, &existing, &seats)
            .await
            .unwrap()
            .is_empty(),
        "missing generated introductions preserve previous text"
    );
    let mut spec = basic();
    spec["folders"][0]["name"] = json!("用户账户");
    spec["folders"][0]["children"][0]["name"] = json!("账户查询");
    let result = commands(spec, &ss, &existing, &seats).await.unwrap();
    let ids: std::collections::HashSet<_> = result
        .iter()
        .filter_map(|c| match c {
            Command::PutFolder { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(ids, std::collections::HashSet::from([root, query]));
    assert!(
        !result
            .iter()
            .any(|c| matches!(c, Command::Place { .. } | Command::RemoveFolder { .. }))
    );
}

#[tokio::test]
async fn a_former_top_level_folder_can_become_a_group_without_losing_its_id() {
    let ss = summaries(2);
    let old = FolderId::new();
    let existing = vec![folder(old, None, "账户", 0)];
    let seats = vec![placement(ss[0].id, old), placement(ss[1].id, old)];
    let spec = json!({"folders":[{"name":"权限中心","summary":"权限相关用途","children":[
        {"existing_id":old,"name":"账户","summary":"账户用途","endpoints":["0","1"]}]}],"unplaced":[]});
    let result = commands(spec, &ss, &existing, &seats).await.unwrap();
    assert!(
        result
            .iter()
            .any(|c| matches!(c,Command::PutFolder{id,parent:Some(_),..} if *id==old))
    );
    assert!(
        !result
            .iter()
            .any(|c| matches!(c, Command::Place { .. } | Command::RemoveFolder { .. }))
    );
}

#[tokio::test]
async fn explicit_rename_and_move_keep_ids_and_stage_equal_named_siblings_safely() {
    let ss = summaries(4);
    let a = FolderId::new();
    let b = FolderId::new();
    let x = FolderId::new();
    let y = FolderId::new();
    let keep_a = FolderId::new();
    let keep_b = FolderId::new();
    let existing = vec![
        folder(a, None, "业务甲", 0),
        folder(x, Some(a), "查询", 0),
        folder(b, None, "业务乙", 1),
        folder(y, Some(b), "查询", 0),
        folder(keep_a, Some(a), "保留功能", 1),
        folder(keep_b, Some(b), "保留功能", 1),
    ];
    let seats = vec![
        placement(ss[0].id, x),
        placement(ss[1].id, y),
        placement(ss[2].id, keep_a),
        placement(ss[3].id, keep_b),
    ];
    let spec = json!({"folders":[
        {"existing_id":a,"name":"业务甲","summary":"业务甲用途","children":[{"existing_id":y,"name":"查询","summary":"查询用途","endpoints":["1"]},{"existing_id":keep_a,"name":"保留功能","summary":"保留功能用途","endpoints":["2"]}]},
        {"existing_id":b,"name":"业务乙","summary":"业务乙用途","children":[{"existing_id":x,"name":"查询","summary":"查询用途","endpoints":["0"]},{"existing_id":keep_b,"name":"保留功能","summary":"保留功能用途","endpoints":["3"]}]}],"unplaced":[]});
    let result = commands(spec, &ss, &existing, &seats).await.unwrap();
    assert_eq!(result.len(), 4);
    assert!(
        matches!(&result[0],Command::PutFolder{parent:None,name,..} if name.starts_with("__curation_"))
    );
    assert!(
        matches!(&result[1],Command::PutFolder{parent:None,name,..} if name.starts_with("__curation_"))
    );
    assert!(
        matches!(&result[2],Command::PutFolder{id,parent:Some(parent),..} if *id==y && *parent==a)
    );
    assert!(
        matches!(&result[3],Command::PutFolder{id,parent:Some(parent),..} if *id==x && *parent==b)
    );
}

#[tokio::test]
async fn manual_folder_metadata_and_placements_are_protected_and_unknown_ownership_is_not_guessed()
{
    let ss = summaries(2);
    let root = FolderId::new();
    let person = Author::Person {
        user: UserId::new(),
    };
    let mut old = folder(root, None, "人工分组", 0);
    old.author = Some(person.clone());
    let mut seat = placement(ss[0].id, root);
    seat.author = person;
    let seats = vec![seat, placement(ss[1].id, root)];
    let spec = json!({"folders":[{"existing_id":root,"name":"人工分组","summary":"人工分组用途","children":[
        {"name":"新增功能","summary":"新增功能用途","endpoints":["0","1"]}]}],"unplaced":[]});
    let result = commands(spec.clone(), &ss, std::slice::from_ref(&old), &seats)
        .await
        .unwrap();
    assert!(
        !result
            .iter()
            .any(|c| matches!(c,Command::PutFolder{id,..} if *id==root))
    );
    assert!(!result.iter().any(|c| matches!(c,Command::Place{endpoint,..}|Command::Unplace{endpoint} if *endpoint==ss[0].id)));
    let mut changed = spec;
    changed["folders"][0]["name"] = json!("模型擅自改名");
    assert!(
        commands(changed.clone(), &ss, std::slice::from_ref(&old), &seats)
            .await
            .is_err()
    );
    old.author = None;
    assert!(commands(changed, &ss, &[old], &seats).await.is_err());
}

#[tokio::test]
async fn retained_links_survive_unclassification_and_an_excessive_final_tree_is_rejected() {
    let ss = summaries(1);
    let root = FolderId::new();
    let old = folder(root, None, "旧目录", 0);
    let result = commands(
        json!({"folders":[],"unplaced":[{"key":"0","reason":"非业务健康检查"}]}),
        &ss,
        &[old],
        &[placement(ss[0].id, root)],
    )
    .await
    .unwrap();
    assert!(matches!(result.as_slice(), [Command::Unplace { .. }]));
    let ss = summaries(13);
    let folders:Vec<_>=(0..13).map(|i| json!({"name":format!("模块{i}"),"summary":"用途","children":[{"name":"功能","summary":"用途","endpoints":[i.to_string()]}]})).collect();
    assert!(
        commands(json!({"folders":folders,"unplaced":[]}), &ss, &[], &[])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_old_shared_id_cannot_be_repurposed_for_an_unrelated_populated_group() {
    let ss = summaries(2);
    let old = FolderId::new();
    let existing = vec![folder(old, None, "原业务", 0)];
    let spec = json!({"folders":[{"existing_id":old,"name":"另一业务","summary":"另一业务用途","children":[
        {"name":"查询","summary":"查询用途","endpoints":["1"]}]}],"unplaced":[{"key":"0","reason":"原接口未分类"}]});
    assert!(
        commands(spec, &ss, &existing, &[placement(ss[0].id, old)])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn prompt_contains_the_complete_inventory_saved_meanings_and_existing_ids() {
    use nexofolio_curate::{CompletionError, Completions, Organize};
    use std::sync::{Arc, Mutex};
    struct Capture(Arc<Mutex<String>>);
    #[async_trait::async_trait]
    impl Completions for Capture {
        async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
            *self.0.lock().unwrap() = prompt.into();
            Ok(json!({"folders":[],"unplaced":(0..47).map(|i| json!({"key":i.to_string(),"reason":"测试明确保留待分类"})).collect::<Vec<_>>()}).to_string())
        }
    }
    let ss = summaries(47);
    let old = folder(FolderId::new(), None, "旧业务", 0);
    let note = EndpointNote {
        endpoint: ss[46].id,
        name: Some("末尾接口名称".into()),
        purpose: Some("末尾接口用途".into()),
        context: EndpointContext {
            description: Some("末尾接口主要输入输出".into()),
            ..Default::default()
        },
        author: author(),
        at: "2026-10-07T00:00:00Z".parse().unwrap(),
    };
    let captured = Arc::new(Mutex::new(String::new()));
    Organize::new(Capture(captured.clone()), "fixed".into())
        .project_with_snapshot(&ss, &[note], std::slice::from_ref(&old), &[])
        .await
        .unwrap();
    let prompt = captured.lock().unwrap();
    assert!(
        prompt.contains("末尾接口名称")
            && prompt.contains("末尾接口用途")
            && prompt.contains("末尾接口主要输入输出")
    );
    assert!(prompt.contains(&old.id.to_string()));
    let start = prompt.find("接口列表:\n").unwrap() + "接口列表:\n".len();
    let data: serde_json::Value = serde_json::Deserializer::from_str(&prompt[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(data.as_array().unwrap().len(), 47);
    assert_eq!(data[46]["key"], "46");
}

use serde_json::{Value, json};
use std::{collections::HashSet, process::Command};

fn violations(metadata: &Value) -> Vec<String> {
    let core = [
        "access",
        "knowledge",
        "intake",
        "triggers",
        "rebuild",
        "evidence",
    ];
    let known = [
        "contracts",
        "evidence",
        "access",
        "knowledge",
        "intake",
        "triggers",
        "rebuild",
        "application",
        "infrastructure",
        "backend",
    ];
    let members: HashSet<_> = metadata["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut errors = Vec::new();
    for package in metadata["packages"].as_array().unwrap() {
        if !members.contains(package["id"].as_str().unwrap()) {
            continue;
        }
        let name = package["name"].as_str().unwrap();
        let short = name.strip_prefix("nexofolio-").unwrap_or(name);
        if !known.contains(&short) {
            errors.push(format!("{name}: no dependency policy defined"));
            continue;
        }
        let allowed_local: Vec<String> = match short {
            "contracts" => vec![],
            s if core.contains(&s) => vec!["nexofolio-contracts".into()],
            "application" => core
                .iter()
                .chain(["contracts"].iter())
                .map(|s| format!("nexofolio-{s}"))
                .collect(),
            "infrastructure" => core
                .iter()
                .chain(["contracts", "application"].iter())
                .map(|s| format!("nexofolio-{s}"))
                .collect(),
            "backend" => [
                "application",
                "infrastructure",
                "contracts",
                "access",
                "intake",
                "knowledge",
            ]
            .iter()
            .map(|s| format!("nexofolio-{s}"))
            .collect(),
            _ => unreachable!(),
        };
        for dependency in package["dependencies"].as_array().unwrap() {
            let dep = dependency["name"].as_str().unwrap();
            let local = dep.starts_with("nexofolio-") || dependency["path"].is_string();
            if local
                && !allowed_local.iter().any(|s| s == dep)
                && !(short == "backend"
                    && ["nexofolio-rebuild", "nexofolio-evidence"].contains(&dep)
                    && dependency["kind"] == "dev")
            {
                errors.push(format!("{name} -> {dep}: forbidden local dependency"));
            }
            if !local && (core.contains(&short) || short == "contracts" || short == "application") {
                let basic = [
                    "async-trait",
                    "serde",
                    "serde_json",
                    "schemars",
                    "thiserror",
                    "uuid",
                    "zeroize",
                ];
                let test_runtime = dep == "tokio" && dependency["kind"] == "dev";
                let intake_pure = ["intake", "knowledge", "evidence", "rebuild", "application"]
                    .contains(&short)
                    && ["sha2", "url", "base64"].contains(&dep);
                if !basic.contains(&dep) && !test_runtime && !intake_pure {
                    errors.push(format!(
                        "{name} -> {dep}: infrastructure dependency in core"
                    ));
                }
            }
        }
    }
    errors
}

fn fixture(owner: &str, dependency: &str, kind: Value) -> Value {
    json!({"workspace_members":[owner],"packages":[{
        "id":owner,"name":owner,"dependencies":[{"name":dependency,"kind":kind}]
    }]})
}

#[test]
fn real_workspace_follows_dependency_policy() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(violations(&metadata), Vec::<String>::new());
}

#[test]
fn forbidden_edges_fail_including_dev_dependencies() {
    for (owner, dep, kind) in [
        ("nexofolio-intake", "nexofolio-rebuild", Value::Null),
        ("nexofolio-triggers", "sqlx", Value::Null),
        ("nexofolio-rebuild", "reqwest", json!("dev")),
        (
            "nexofolio-application",
            "nexofolio-infrastructure",
            Value::Null,
        ),
        ("nexofolio-intake", "nexofolio-triggers", json!("dev")),
    ] {
        assert!(
            !violations(&fixture(owner, dep, kind)).is_empty(),
            "edge was accepted: {owner}->{dep}"
        );
    }
}

#[test]
fn declared_ports_and_test_runtime_are_allowed() {
    assert!(
        violations(&fixture(
            "nexofolio-intake",
            "nexofolio-contracts",
            Value::Null
        ))
        .is_empty()
    );
    assert!(violations(&fixture("nexofolio-intake", "tokio", json!("dev"))).is_empty());
    assert!(!violations(&fixture("nexofolio-intake", "tokio", Value::Null)).is_empty());
    assert!(!violations(&fixture("nexofolio-surprise", "serde", Value::Null)).is_empty());
}

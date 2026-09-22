//! Boundary checks apply to normal, build and dev dependencies alike.
use serde_json::{Value, json};
use std::{collections::HashSet, process::Command};

fn allowed_local(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "nexofolio-common" => Some(&[]),
        "nexofolio-access-contracts" => Some(&["nexofolio-common"]),
        "nexofolio-access" => Some(&["nexofolio-common", "nexofolio-access-contracts"]),
        "nexofolio-access-adapter" => Some(&["nexofolio-common", "nexofolio-access-contracts"]),
        "nexofolio-backend" => Some(&[
            "nexofolio-common",
            "nexofolio-access-contracts",
            "nexofolio-access",
            "nexofolio-access-adapter",
        ]),
        _ => None,
    }
}
fn violations(metadata: &Value) -> Vec<String> {
    let members: HashSet<_> = metadata["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut errors = vec![];
    for package in metadata["packages"].as_array().unwrap() {
        if !members.contains(package["id"].as_str().unwrap()) {
            continue;
        }
        let name = package["name"].as_str().unwrap();
        let Some(allowed) = allowed_local(name) else {
            errors.push(format!("{name}: undefined module boundary"));
            continue;
        };
        for dependency in package["dependencies"].as_array().unwrap() {
            let dep = dependency["name"].as_str().unwrap();
            if dep.starts_with("nexofolio-") || dependency["path"].is_string() {
                if !allowed.contains(&dep) {
                    errors.push(format!("{name} -> {dep}: forbidden dependency"));
                }
            } else if [
                "nexofolio-common",
                "nexofolio-access-contracts",
                "nexofolio-access",
            ]
            .contains(&name)
            {
                let pure = [
                    "async-trait",
                    "serde",
                    "serde_json",
                    "schemars",
                    "thiserror",
                    "uuid",
                    "zeroize",
                ];
                if !(pure.contains(&dep) || dep == "tokio" && dependency["kind"] == "dev") {
                    errors.push(format!(
                        "{name} -> {dep}: IO/SDK dependency in a contract or core"
                    ));
                }
            } else if name == "nexofolio-backend" && dep == "sqlx" && dependency["kind"] != "dev" {
                errors.push("transport/composition cannot access database tables".into());
            }
        }
    }
    errors
}
fn fixture(owner: &str, dep: &str, kind: Value) -> Value {
    json!({"workspace_members":[owner],"packages":[{"id":owner,"name":owner,"dependencies":[{"name":dep,"kind":kind}]}]})
}
#[test]
fn workspace_dependencies_are_explicit_and_directional() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        violations(&serde_json::from_slice(&output.stdout).unwrap()),
        Vec::<String>::new()
    );
}
#[test]
fn forbidden_edges_fail_even_through_test_or_build_dependencies() {
    for kind in [Value::Null, json!("dev"), json!("build")] {
        for (owner, dep) in [
            ("nexofolio-access", "nexofolio-access-adapter"),
            ("nexofolio-access-adapter", "nexofolio-access"),
            ("nexofolio-access-contracts", "nexofolio-access"),
            ("nexofolio-common", "nexofolio-access-contracts"),
            ("nexofolio-access", "nexofolio-rebuild"),
            ("nexofolio-access-contracts", "sqlx"),
            ("nexofolio-access", "reqwest"),
            ("nexofolio-surprise", "serde"),
        ] {
            assert!(
                !violations(&fixture(owner, dep, kind.clone())).is_empty(),
                "accepted {owner} -> {dep}"
            );
        }
    }
    let mut renamed = fixture("nexofolio-access", "nexofolio-access-adapter", Value::Null);
    renamed["packages"][0]["dependencies"][0]["rename"] = json!("innocent_name");
    assert!(!violations(&renamed).is_empty());
    assert!(!violations(&fixture("nexofolio-backend", "sqlx", Value::Null)).is_empty());
}
#[test]
fn public_contract_dependencies_are_allowed() {
    for owner in ["nexofolio-access", "nexofolio-access-adapter"] {
        assert!(violations(&fixture(owner, "nexofolio-access-contracts", Value::Null)).is_empty());
    }
}

//! Boundary checks apply to normal, build and dev dependencies alike.
//!
//! Every workspace crate has a role derived from its name. Modules are listed in
//! [`MODULES`]; a module `m` consists of `nexofolio-m-contracts`,
//! `nexofolio-m` (core) and `nexofolio-m-adapter`. Modules never depend on one
//! another: they share only `nexofolio-contracts` and `nexofolio-common`, and
//! only `nexofolio-backend` wires them together.
use serde_json::{Value, json};
use std::{collections::HashSet, process::Command};

mod sql_ownership;

/// Registered backend modules. A crate or directory outside this list is an
/// undefined boundary.
pub(crate) const MODULES: &[&str] = &["access"];

const COMMON: &str = "nexofolio-common";
const CONTRACTS: &str = "nexofolio-contracts";
const BACKEND: &str = "nexofolio-backend";

/// External crates allowed in anything that must stay free of IO.
const PURE: &[&str] = &[
    "async-trait",
    "chrono",
    "serde",
    "serde_json",
    "schemars",
    "thiserror",
    "uuid",
    "zeroize",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Role {
    Common,
    Contracts,
    ModuleContracts(String),
    ModuleCore(String),
    ModuleAdapter(String),
    Backend,
}

fn role(name: &str, modules: &[&str]) -> Option<Role> {
    match name {
        COMMON => return Some(Role::Common),
        CONTRACTS => return Some(Role::Contracts),
        BACKEND => return Some(Role::Backend),
        _ => {}
    }
    modules.iter().find_map(|module| {
        let core = format!("nexofolio-{module}");
        if name == core {
            Some(Role::ModuleCore(module.to_string()))
        } else if name == format!("{core}-contracts") {
            Some(Role::ModuleContracts(module.to_string()))
        } else if name == format!("{core}-adapter") {
            Some(Role::ModuleAdapter(module.to_string()))
        } else {
            None
        }
    })
}

fn allowed_local(role: &Role, modules: &[&str]) -> Vec<String> {
    let base = |extra: &[String]| {
        let mut allowed = vec![COMMON.to_string(), CONTRACTS.to_string()];
        allowed.extend_from_slice(extra);
        allowed
    };
    match role {
        Role::Common => vec![],
        Role::Contracts => vec![COMMON.to_string()],
        Role::ModuleContracts(_) => base(&[]),
        Role::ModuleCore(m) | Role::ModuleAdapter(m) => base(&[format!("nexofolio-{m}-contracts")]),
        Role::Backend => {
            let mut allowed = base(&[]);
            for m in modules {
                allowed.push(format!("nexofolio-{m}-contracts"));
                allowed.push(format!("nexofolio-{m}"));
                allowed.push(format!("nexofolio-{m}-adapter"));
            }
            allowed
        }
    }
}

fn must_stay_pure(role: &Role) -> bool {
    matches!(
        role,
        Role::Common | Role::Contracts | Role::ModuleContracts(_) | Role::ModuleCore(_)
    )
}

fn violations(metadata: &Value, modules: &[&str]) -> Vec<String> {
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
        let Some(role) = role(name, modules) else {
            errors.push(format!("{name}: undefined module boundary"));
            continue;
        };
        let allowed = allowed_local(&role, modules);
        for dependency in package["dependencies"].as_array().unwrap() {
            let dep = dependency["name"].as_str().unwrap();
            let is_dev = dependency["kind"] == "dev";
            if dep.starts_with("nexofolio-") || dependency["path"].is_string() {
                if !allowed.iter().any(|a| a == dep) {
                    errors.push(format!("{name} -> {dep}: forbidden dependency"));
                }
            } else if must_stay_pure(&role) {
                if !(PURE.contains(&dep) || dep == "tokio" && is_dev) {
                    errors.push(format!(
                        "{name} -> {dep}: IO/SDK dependency in a contract or core"
                    ));
                }
            } else if role == Role::Backend && dep == "sqlx" && !is_dev {
                errors.push("transport/composition cannot access database tables".into());
            }
        }
    }
    errors
}

fn fixture(owner: &str, dep: &str, kind: Value) -> Value {
    json!({"workspace_members":[owner],"packages":[{"id":owner,"name":owner,"dependencies":[{"name":dep,"kind":kind}]}]})
}

/// A wider registry so cross-module rules are exercised, not just "unknown crate".
const FIXTURE_MODULES: &[&str] = &["access", "intake", "observe"];

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
        violations(&serde_json::from_slice(&output.stdout).unwrap(), MODULES),
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
            ("nexofolio-common", "nexofolio-contracts"),
            ("nexofolio-access", "nexofolio-rebuild"),
            ("nexofolio-access-contracts", "sqlx"),
            ("nexofolio-access", "reqwest"),
            ("nexofolio-surprise", "serde"),
            // The shared contract layer knows no module.
            ("nexofolio-contracts", "nexofolio-access-contracts"),
            ("nexofolio-contracts", "nexofolio-observe"),
            ("nexofolio-contracts", "sqlx"),
            ("nexofolio-contracts", "reqwest"),
            // Modules never reach into each other, at any layer.
            ("nexofolio-intake", "nexofolio-observe"),
            ("nexofolio-intake", "nexofolio-observe-contracts"),
            ("nexofolio-intake-adapter", "nexofolio-observe-adapter"),
            ("nexofolio-observe-contracts", "nexofolio-access-contracts"),
            ("nexofolio-observe", "nexofolio-access-contracts"),
            ("nexofolio-intake-contracts", "nexofolio-intake"),
            ("nexofolio-observe", "sqlx"),
        ] {
            assert!(
                !violations(&fixture(owner, dep, kind.clone()), FIXTURE_MODULES).is_empty(),
                "accepted {owner} -> {dep} ({kind})"
            );
        }
    }
    let mut renamed = fixture("nexofolio-access", "nexofolio-access-adapter", Value::Null);
    renamed["packages"][0]["dependencies"][0]["rename"] = json!("innocent_name");
    assert!(!violations(&renamed, FIXTURE_MODULES).is_empty());
    assert!(!violations(&fixture(BACKEND, "sqlx", Value::Null), FIXTURE_MODULES).is_empty());

    let mut local_path = fixture("nexofolio-observe", "helper", Value::Null);
    local_path["packages"][0]["dependencies"][0]["path"] = json!("../../shared/helper");
    assert!(
        !violations(&local_path, FIXTURE_MODULES).is_empty(),
        "unlisted path crate"
    );

    // A module that is not registered is an undefined boundary, even if its edges look fine.
    assert!(
        !violations(
            &fixture("nexofolio-curate", CONTRACTS, Value::Null),
            FIXTURE_MODULES
        )
        .is_empty()
    );
}

#[test]
fn public_contract_dependencies_are_allowed() {
    for owner in ["nexofolio-access", "nexofolio-access-adapter"] {
        assert!(
            violations(
                &fixture(owner, "nexofolio-access-contracts", Value::Null),
                MODULES
            )
            .is_empty()
        );
    }
    for kind in [Value::Null, json!("dev")] {
        for owner in [
            "nexofolio-access-contracts",
            "nexofolio-intake",
            "nexofolio-intake-adapter",
            "nexofolio-observe",
            BACKEND,
        ] {
            for dep in [COMMON, CONTRACTS] {
                assert!(
                    violations(&fixture(owner, dep, kind.clone()), FIXTURE_MODULES).is_empty(),
                    "rejected {owner} -> {dep}"
                );
            }
        }
    }
    assert!(violations(&fixture(CONTRACTS, COMMON, Value::Null), MODULES).is_empty());
    assert!(violations(&fixture(CONTRACTS, "chrono", Value::Null), MODULES).is_empty());
    assert!(violations(&fixture(CONTRACTS, "tokio", json!("dev")), MODULES).is_empty());
    assert!(
        violations(
            &fixture("nexofolio-observe-adapter", "sqlx", Value::Null),
            FIXTURE_MODULES
        )
        .is_empty()
    );
    for dep in [
        "nexofolio-intake",
        "nexofolio-intake-adapter",
        "nexofolio-observe-contracts",
    ] {
        assert!(violations(&fixture(BACKEND, dep, Value::Null), FIXTURE_MODULES).is_empty());
    }
}

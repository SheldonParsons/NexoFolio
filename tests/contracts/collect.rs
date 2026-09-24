//! Guards the public collect contract in `contracts/collect/v1`.
//!
//! Every fixture must match its expected outcome, and `manifest.json` must pin
//! exactly the files on disk, because clients copy the directory and verify the
//! same hashes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

const SCHEMAS: [&str; 3] = ["batch", "receipt", "error"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/collect/v1")
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn validator(schema: &str) -> jsonschema::Validator {
    let schema = read_json(&root().join(format!("{schema}.schema.json")));
    jsonschema::draft202012::meta::validate(&schema).expect("schema is valid draft 2020-12");
    jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .expect("schema compiles")
}

fn fixtures(schema: &str, outcome: &str) -> Vec<PathBuf> {
    let dir = root().join("fixtures").join(schema).join(outcome);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .map(|entry| entry.expect("readable fixture dir").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
}

fn files_on_disk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("readable contract dir") {
        let path = entry.expect("readable entry").path();
        if path.is_dir() {
            files_on_disk(&path, base, out);
        } else if path.file_name().is_some_and(|name| name != "manifest.json") {
            let relative = path.strip_prefix(base).expect("inside base");
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn valid_fixtures_pass_their_schema() {
    for schema in SCHEMAS {
        let validator = validator(schema);
        let paths = fixtures(schema, "valid");
        assert!(!paths.is_empty(), "{schema} has no valid fixtures");
        for path in paths {
            let errors: Vec<String> = validator
                .iter_errors(&read_json(&path))
                .map(|e| format!("{} at {}", e, e.instance_path()))
                .collect();
            assert!(
                errors.is_empty(),
                "{} should pass: {errors:?}",
                path.display()
            );
        }
    }
}

#[test]
fn invalid_fixtures_fail_their_schema() {
    for schema in SCHEMAS {
        let validator = validator(schema);
        for path in fixtures(schema, "invalid") {
            assert!(
                !validator.is_valid(&read_json(&path)),
                "{} should fail",
                path.display()
            );
        }
    }
    assert!(
        fixtures("batch", "invalid").len() >= 10,
        "batch needs negative coverage"
    );
}

#[test]
fn manifest_pins_every_contract_file() {
    let root = root();
    let manifest = read_json(&root.join("manifest.json"));
    let pinned: BTreeMap<String, String> =
        serde_json::from_value(manifest["files"].clone()).expect("manifest.files is a map");

    let mut on_disk = Vec::new();
    files_on_disk(&root, &root, &mut on_disk);
    on_disk.sort();

    let mut problems = Vec::new();
    for name in &on_disk {
        let digest = Sha256::digest(fs::read(root.join(name)).expect("readable contract file"));
        let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        match pinned.get(name) {
            None => problems.push(format!("not pinned: \"{name}\": \"{actual}\"")),
            Some(expected) if *expected != actual => {
                problems.push(format!("stale hash: \"{name}\": \"{actual}\""));
            }
            Some(_) => {}
        }
    }
    for name in pinned.keys() {
        if !on_disk.contains(name) {
            problems.push(format!("pinned but missing: {name}"));
        }
    }
    assert!(
        problems.is_empty(),
        "update manifest.json:\n{}",
        problems.join("\n")
    );
}

#[test]
fn manifest_limits_match_the_schema() {
    let manifest = read_json(&root().join("manifest.json"));
    let batch = read_json(&root().join("batch.schema.json"));
    let receipt = read_json(&root().join("receipt.schema.json"));
    let records = &manifest["limits"]["records_per_batch"];

    assert_eq!(records, &batch["properties"]["records"]["maxItems"]);
    assert_eq!(records, &receipt["properties"]["accepted"]["maximum"]);
    assert_eq!(records, &receipt["properties"]["rejected"]["maxItems"]);

    let kinds: Vec<&str> = manifest["kinds"]
        .as_object()
        .expect("kinds map")
        .keys()
        .map(String::as_str)
        .collect();
    let mut schema_kinds: Vec<&str> = batch["$defs"]["Record"]["properties"]["kind"]["enum"]
        .as_array()
        .expect("kind enum")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    schema_kinds.sort_unstable();
    assert_eq!(
        kinds, schema_kinds,
        "manifest.kinds (sorted) vs schema kind enum"
    );
}

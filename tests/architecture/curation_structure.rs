//! Guard the responsibilities established by the curation refactor.
use std::{
    fs,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn sources(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn curation_http_does_not_execute_passes_or_schedule_workers() {
    let source = fs::read_to_string(root().join("apps/backend/src/http/curate.rs")).unwrap();
    for forbidden in [
        "tokio::",
        "KnowledgeWriter",
        "KnowledgeReader",
        "Completions",
        "JoinSet",
        ".apply(",
        ".complete(",
        "field_inputs(",
        "pending_fields(",
        ".endpoint(&",
        "workflow()",
    ] {
        assert!(
            !source.contains(forbidden),
            "curation HTTP must only authorize and dispatch: {forbidden}"
        );
    }
}

#[test]
fn evidence_and_rules_cannot_call_models_or_persistence_ports() {
    let root = root();
    for directory in ["evidence", "rules"] {
        let mut files = Vec::new();
        sources(
            &root.join("modules/curate/core/src").join(directory),
            &mut files,
        );
        for file in files {
            let source = fs::read_to_string(&file).unwrap();
            for forbidden in [
                "Completions",
                "KnowledgeWriter",
                "KnowledgeReader",
                "EndpointReader",
                "Command::",
                ".complete(",
                "interpretation::",
                "tokio::",
                "sqlx::",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{} crosses a pure evidence/rule boundary: {forbidden}",
                    file.display()
                );
            }
        }
    }
}

#[test]
fn curation_and_storage_files_remain_reviewable() {
    let root = root();
    for directory in [
        "modules/curate/core/src",
        "modules/knowledge/adapter/src",
        "apps/backend/src/runtime/curation",
    ] {
        let mut files = Vec::new();
        sources(&root.join(directory), &mut files);
        for file in files {
            if file.file_name().is_some_and(|name| name == "tests.rs") {
                continue;
            }
            let lines = fs::read_to_string(&file).unwrap().lines().count();
            assert!(
                lines <= 400,
                "{} has {lines} lines; review responsibilities before increasing the limit",
                file.display()
            );
        }
    }
}

//! Each module owns one PostgreSQL schema named after it and touches nothing else.
//!
//! A module's connections put only its own schema on the `search_path`, so an
//! unqualified name always means the module's own table. The check reads every
//! migration and every SQL string literal in a module's Rust sources, then
//! rejects:
//! - schema-qualified names pointing at another module's schema or at `public`;
//! - creating any schema but its own, or a table qualified with another schema;
//! - directories under `modules/` that are not registered modules.
use std::fs;
use std::path::{Path, PathBuf};

use super::MODULES;

/// Words that may sit between `CREATE TABLE` and the table name.
const TABLE_MODIFIERS: &[&str] = &[
    "only",
    "if",
    "not",
    "exists",
    "lateral",
    "unlogged",
    "temporary",
    "temp",
];

#[derive(Debug, Clone)]
struct SqlSource {
    module: String,
    origin: String,
    sql: String,
}

fn tokens(sql: &str) -> Vec<String> {
    let mut without_comments = String::with_capacity(sql.len());
    for line in sql.lines() {
        without_comments.push_str(line.split("--").next().unwrap_or_default());
        without_comments.push('\n');
    }
    without_comments
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '"'))
        .map(|token| token.replace('"', ""))
        .filter(|token| !token.is_empty())
        .collect()
}

fn split_qualified(name: &str) -> (Option<&str>, &str) {
    match name.split_once('.') {
        Some((schema, table)) => (Some(schema), table),
        None => (None, name),
    }
}

fn sql_violations(sources: &[SqlSource], modules: &[&str]) -> Vec<String> {
    let mut errors = Vec::new();
    for source in sources {
        let module = source.module.as_str();
        let toks = tokens(&source.sql);
        let at = |i: usize| toks.get(i).map(String::as_str).unwrap_or_default();
        let mut report =
            |message: String| errors.push(format!("{} ({module}): {message}", source.origin));

        for (i, token) in toks.iter().enumerate() {
            if let (Some(schema), _) = split_qualified(token)
                && schema != module
                && (schema == "public" || modules.contains(&schema))
            {
                report(format!("reaches outside schema `{module}`: {token}"));
            }

            if token == "create" && at(i + 1) == "schema" {
                let name = name_after(&toks, i + 2);
                if name != module {
                    report(format!("creates schema `{name}`"));
                }
            }

            if token == "create" && at(i + 1) == "table" {
                let name = name_after(&toks, i + 2);
                if split_qualified(name)
                    .0
                    .is_some_and(|schema| schema != module)
                {
                    report(format!("creates table outside schema `{module}`: {name}"));
                }
            }
        }
    }
    errors
}

/// The first identifier at or after `start` that is not a modifier such as `IF NOT EXISTS`.
fn name_after(toks: &[String], start: usize) -> &str {
    toks[start.min(toks.len())..]
        .iter()
        .map(String::as_str)
        .find(|t| !TABLE_MODIFIERS.contains(t))
        .unwrap_or_default()
}

/// String literals in Rust source, which is where adapters keep their SQL.
/// Handles escapes and raw strings; skips comments and char literals.
fn rust_string_literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut literals = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            '\'' => {
                // Char literal ('x', '\n', '"'); anything else is a lifetime.
                if chars.get(i + 1) == Some(&'\\') {
                    // Skip the quote, the backslash and the escaped char, then find the close.
                    i += 3;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            'r' if matches!(chars.get(i + 1), Some('"' | '#'))
                && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')) =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while chars.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if chars.get(j) != Some(&'"') {
                    i += 1;
                    continue;
                }
                let start = j + 1;
                let mut k = start;
                loop {
                    if k >= chars.len() {
                        break;
                    }
                    if chars[k] == '"' && (1..=hashes).all(|h| chars.get(k + h) == Some(&'#')) {
                        break;
                    }
                    k += 1;
                }
                literals.push(chars[start..k.min(chars.len())].iter().collect());
                i = k + 1 + hashes;
            }
            '"' => {
                let mut literal = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    if let Some(c) = chars.get(i) {
                        literal.push(*c);
                    }
                    i += 1;
                }
                literals.push(literal);
                i += 1;
            }
            _ => i += 1,
        }
    }
    literals
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("readable entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                collect_files(&path, out);
            }
        } else if path
            .extension()
            .is_some_and(|ext| ext == "sql" || ext == "rs")
        {
            out.push(path);
        }
    }
}

fn workspace_sources(root: &Path) -> (Vec<SqlSource>, Vec<String>) {
    let mut sources = Vec::new();
    let mut unregistered = Vec::new();
    for entry in fs::read_dir(root.join("modules")).expect("modules dir") {
        let path = entry.expect("readable entry").path();
        if !path.is_dir() {
            continue;
        }
        let module = path.file_name().unwrap().to_string_lossy().into_owned();
        if !MODULES.contains(&module.as_str()) {
            unregistered.push(format!("modules/{module}: unregistered module directory"));
            continue;
        }
        let mut files = Vec::new();
        collect_files(&path, &mut files);
        for file in files {
            let text = fs::read_to_string(&file).expect("readable source");
            let origin = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .display()
                .to_string();
            let sql = if file.extension().is_some_and(|ext| ext == "rs") {
                rust_string_literals(&text).join("\n")
            } else {
                text
            };
            sources.push(SqlSource {
                module: module.clone(),
                origin,
                sql,
            });
        }
    }
    (sources, unregistered)
}

#[test]
fn modules_only_touch_their_own_schema() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (sources, unregistered) = workspace_sources(&root);
    assert!(
        sources.iter().any(|s| s.origin.ends_with(".sql")),
        "migrations were scanned"
    );
    let mut errors = unregistered;
    errors.extend(sql_violations(&sources, MODULES));
    assert_eq!(errors, Vec::<String>::new());
}

fn source(module: &str, sql: &str) -> SqlSource {
    SqlSource {
        module: module.into(),
        origin: format!("fixture:{module}"),
        sql: sql.into(),
    }
}

const FIXTURE_MODULES: &[&str] = &["access", "intake", "observe"];

fn check(sources: &[SqlSource]) -> Vec<String> {
    sql_violations(sources, FIXTURE_MODULES)
}

#[test]
fn own_schema_sql_is_allowed() {
    let clean = [
        source(
            "observe",
            r#"CREATE SCHEMA IF NOT EXISTS observe;
               CREATE TABLE observe.endpoints (id uuid PRIMARY KEY, project_id uuid NOT NULL);
               CREATE TABLE "observe"."samples" (endpoint_id uuid REFERENCES observe.endpoints(id));
               -- project_id comes from access, by ID only: FROM projects is fine in a comment
               WITH recent AS (SELECT * FROM observe.samples) SELECT e.id FROM observe.endpoints e JOIN recent r ON true;
               INSERT INTO observe.endpoints (id, project_id) VALUES ($1, $2);
               UPDATE observe.endpoints SET project_id = $1;"#,
        ),
        source(
            "intake",
            "CREATE SCHEMA intake; CREATE UNLOGGED TABLE intake.batches (id uuid);",
        ),
        // Unqualified names resolve through the module's own search_path.
        source(
            "intake",
            "CREATE TABLE receipts (batch_id uuid REFERENCES batches(id)); SELECT * FROM receipts JOIN intake.batches b ON true;",
        ),
    ];
    assert_eq!(check(&clean), Vec::<String>::new());
}

#[test]
fn cross_module_sql_is_rejected() {
    for (module, sql) in [
        ("observe", "CREATE TABLE public.endpoints (id uuid)"),
        ("observe", "CREATE TABLE intake.endpoints (id uuid)"),
        ("observe", "CREATE SCHEMA intake"),
        ("observe", "SELECT * FROM intake.batches"),
        (
            "observe",
            "SELECT * FROM observe.endpoints e JOIN intake.batches b ON b.id = e.id",
        ),
        ("observe", "INSERT INTO intake.batches VALUES ($1)"),
        ("observe", "UPDATE intake.batches SET id = $1"),
        ("observe", "DELETE FROM intake.batches"),
        ("observe", "SELECT name FROM public.projects"),
        (
            "observe",
            "SELECT \"intake\".\"batches\".id FROM observe.endpoints",
        ),
        ("intake", "SELECT count(*) FROM ONLY observe.endpoints"),
        ("access", "SELECT * FROM observe.endpoints"),
        ("access", "CREATE TABLE observe.stolen (id uuid)"),
        ("access", "CREATE TABLE public.stray (id uuid)"),
        ("access", "SELECT * FROM public.users"),
    ] {
        assert!(
            !check(&[source(module, sql)]).is_empty(),
            "accepted {module}: {sql}"
        );
    }
}

#[test]
fn rust_string_literals_are_extracted() {
    let code = r####"
        // sqlx::query("SELECT * FROM commented_out")
        /* "SELECT * FROM block_comment" */
        fn f<'a>(x: &'a str) -> char { let q = '"'; let e = '\''; '\n' }
        let a = sqlx::query("SELECT \"id\" FROM observe.endpoints WHERE a = $1");
        let b = sqlx::query(r#"INSERT INTO intake.batches VALUES ("x")"#);
        let c = r"raw";
    "####;
    let literals = rust_string_literals(code);
    assert_eq!(
        literals,
        vec![
            "SELECT \"id\" FROM observe.endpoints WHERE a = $1",
            "INSERT INTO intake.batches VALUES (\"x\")",
            "raw",
        ]
    );
}

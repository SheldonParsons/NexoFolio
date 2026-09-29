use std::fmt::Display;
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{
    DatabaseProbe, EndpointId, Error, FolderId, LinkId, ProjectId, Result, RoundId, Secret,
};
use nexofolio_contracts::knowledge::{
    Author, EndpointNote, FieldNote, Link, Relation, Revision, Round, RoundState, Target,
};
use nexofolio_knowledge_contracts::{
    FolderRow, KnowledgeStore, KnowledgeTx, LinkIdentity, PlacementRow, StoreError, StoreResult,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow};
use sqlx::{ConnectOptions, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

/// Everything knowledge stores lives in this schema. Connections put it alone
/// on the `search_path`, so SQL in this crate stays unqualified.
const SCHEMA: &str = "knowledge";

/// Advisory lock key (first half): one per project.
const PROJECT_LOCK: i32 = 0x6b6e_0001;

#[derive(Clone)]
pub struct PostgresKnowledge {
    pool: PgPool,
}

impl PostgresKnowledge {
    /// Lazy connections keep liveness available during database outages.
    pub fn new(url: &Secret, max_connections: u32, timeout: Duration) -> Result<Self> {
        if max_connections == 0 || timeout.is_zero() {
            return Err(Error::invalid("database pool limits must be positive"));
        }
        if !url.expose().starts_with("postgres://") && !url.expose().starts_with("postgresql://") {
            return Err(Error::invalid("DATABASE_URL must be a PostgreSQL URL"));
        }
        let options = PgConnectOptions::from_str(url.expose())
            .map_err(|_| Error::invalid("DATABASE_URL is invalid"))?
            .options([("search_path", SCHEMA)])
            .disable_statement_logging();
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(timeout)
            .connect_lazy_with(options);
        Ok(Self { pool })
    }

    /// Explicit administration only; the api never migrates automatically.
    pub async fn migrate(&self) -> Result<()> {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.create_schema(SCHEMA);
        migrator.dangerous_set_table_name(format!("{SCHEMA}._sqlx_migrations"));
        migrator
            .run(&self.pool)
            .await
            .map_err(|_| Error::Unavailable {
                component: "database_migrations",
            })
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[async_trait]
impl DatabaseProbe for PostgresKnowledge {
    async fn check(&self) -> Result<()> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map(|_| ())
            .map_err(|_| Error::Unavailable {
                component: "database",
            })
    }
}

fn unavailable<E>(_: E) -> StoreError {
    StoreError::Unavailable
}

/// IDs are UUIDs underneath but do not expose them.
fn uuid(id: impl Display) -> Uuid {
    Uuid::parse_str(&id.to_string()).expect("identifiers are UUIDs")
}

fn id<T: FromStr>(uuid: Uuid) -> T {
    T::from_str(&uuid.to_string())
        .ok()
        .expect("identifiers are UUIDs")
}

/// Unit enums are stored as their serde name.
fn text<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => unreachable!("stored enums serialise to strings"),
    }
}

fn parse<T: DeserializeOwned>(name: String) -> StoreResult<T> {
    serde_json::from_value(Value::String(name)).map_err(unavailable)
}

/// Authors are stored whole: they carry which pass and which model wrote a value.
fn author(value: Value) -> StoreResult<Author> {
    serde_json::from_value(value).map_err(unavailable)
}

fn json<T: Serialize>(value: &T) -> StoreResult<Value> {
    serde_json::to_value(value).map_err(unavailable)
}

#[async_trait]
impl KnowledgeStore for PostgresKnowledge {
    async fn begin(&self) -> StoreResult<Box<dyn KnowledgeTx>> {
        let tx = self.pool.begin().await.map_err(unavailable)?;
        Ok(Box::new(PostgresTx { tx }))
    }
}

struct PostgresTx {
    tx: Transaction<'static, Postgres>,
}

// Macros rather than consts: sqlx only takes SQL that is a literal.
macro_rules! round_columns {
    () => {
        "id, project_id, author, started_at, finished_at, state, summary"
    };
}
macro_rules! folder_columns {
    () => {
        "id, project_id, parent_id, name, summary, position"
    };
}
macro_rules! placement_columns {
    () => {
        "endpoint_id, project_id, folder_id, author, at"
    };
}
macro_rules! endpoint_note_columns {
    () => {
        "endpoint_id, name, purpose, author, at"
    };
}
macro_rules! field_note_columns {
    () => {
        "endpoint_id, path, text, values_seen, author, at"
    };
}
macro_rules! link_columns {
    () => {
        "id, from_endpoint, from_field, to_endpoint, to_field, relation, evidence, author, at"
    };
}

fn round_row(row: &PgRow) -> StoreResult<Round> {
    Ok(Round {
        id: id(row.get("id")),
        project_id: id(row.get("project_id")),
        author: author(row.get("author"))?,
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        state: parse(row.get("state"))?,
        summary: row.get("summary"),
    })
}

fn folder_row(row: &PgRow) -> FolderRow {
    FolderRow {
        id: id(row.get("id")),
        project_id: id(row.get("project_id")),
        parent: row.get::<Option<Uuid>, _>("parent_id").map(id),
        name: row.get("name"),
        summary: row.get("summary"),
        position: row.get("position"),
    }
}

fn placement_row(row: &PgRow) -> StoreResult<PlacementRow> {
    Ok(PlacementRow {
        endpoint: id(row.get("endpoint_id")),
        project_id: id(row.get("project_id")),
        folder: id(row.get("folder_id")),
        author: author(row.get("author"))?,
        at: row.get("at"),
    })
}

fn endpoint_note_row(row: &PgRow) -> StoreResult<EndpointNote> {
    Ok(EndpointNote {
        endpoint: id(row.get("endpoint_id")),
        name: row.get("name"),
        purpose: row.get("purpose"),
        author: author(row.get("author"))?,
        at: row.get("at"),
    })
}

fn field_note_row(row: &PgRow) -> StoreResult<FieldNote> {
    Ok(FieldNote {
        endpoint: id(row.get("endpoint_id")),
        path: row.get("path"),
        text: row.get("text"),
        values: serde_json::from_value(row.get("values_seen")).map_err(unavailable)?,
        author: author(row.get("author"))?,
        at: row.get("at"),
    })
}

#[async_trait]
impl KnowledgeTx for PostgresTx {
    async fn commit(self: Box<Self>) -> StoreResult<()> {
        self.tx.commit().await.map_err(unavailable)
    }

    async fn lock_project(&mut self, project: ProjectId) -> StoreResult<()> {
        sqlx::query("SELECT pg_advisory_xact_lock($1,hashtext($2))")
            .bind(PROJECT_LOCK)
            .bind(project.to_string())
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    // -- rounds ---------------------------------------------------------------

    async fn create_round(&mut self, round: &Round) -> StoreResult<()> {
        sqlx::query(concat!(
            "INSERT INTO rounds (",
            round_columns!(),
            ") VALUES ($1,$2,$3,$4,$5,$6,$7)"
        ))
        .bind(uuid(round.id))
        .bind(uuid(round.project_id))
        .bind(json(&round.author)?)
        .bind(round.started_at)
        .bind(round.finished_at)
        .bind(text(round.state))
        .bind(round.summary.as_deref())
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn round(&mut self, round: RoundId) -> StoreResult<Option<Round>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            round_columns!(),
            " FROM rounds WHERE id = $1"
        ))
        .bind(uuid(round))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(round_row).transpose()
    }

    async fn finish_round(
        &mut self,
        round: RoundId,
        state: RoundState,
        summary: Option<&str>,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        sqlx::query("UPDATE rounds SET state = $2, summary = $3, finished_at = $4 WHERE id = $1")
            .bind(uuid(round))
            .bind(text(state))
            .bind(summary)
            .bind(at)
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    async fn rounds(&mut self, project: ProjectId, limit: usize) -> StoreResult<Vec<Round>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            round_columns!(),
            " FROM rounds WHERE project_id = $1 ORDER BY started_at DESC LIMIT $2"
        ))
        .bind(uuid(project))
        .bind(limit as i64)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(round_row).collect()
    }

    async fn record_revision(&mut self, revision: &Revision) -> StoreResult<()> {
        sqlx::query("INSERT INTO revisions (round_id, seq, target, before) VALUES ($1,$2,$3,$4)")
            .bind(uuid(revision.round))
            .bind(revision.seq)
            .bind(json(&revision.target)?)
            .bind(revision.before.clone())
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    async fn revisions(&mut self, round: RoundId) -> StoreResult<Vec<Revision>> {
        let rows = sqlx::query(
            "SELECT seq, target, before FROM revisions WHERE round_id = $1 ORDER BY seq DESC",
        )
        .bind(uuid(round))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter()
            .map(|row| {
                Ok(Revision {
                    round,
                    seq: row.get("seq"),
                    target: serde_json::from_value(row.get("target")).map_err(unavailable)?,
                    before: row.get("before"),
                })
            })
            .collect()
    }

    // -- folders --------------------------------------------------------------

    async fn folder(&mut self, id: FolderId) -> StoreResult<Option<FolderRow>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            folder_columns!(),
            " FROM folders WHERE id = $1"
        ))
        .bind(uuid(id))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        Ok(row.as_ref().map(folder_row))
    }

    async fn folders(&mut self, project: ProjectId) -> StoreResult<Vec<FolderRow>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            folder_columns!(),
            " FROM folders WHERE project_id = $1"
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        Ok(rows.iter().map(folder_row).collect())
    }

    async fn put_folder(&mut self, folder: &FolderRow) -> StoreResult<()> {
        sqlx::query(concat!(
            "INSERT INTO folders (",
            folder_columns!(),
            ") VALUES ($1,$2,$3,$4,$5,$6)",
            " ON CONFLICT (id) DO UPDATE SET parent_id = $3, name = $4,",
            " summary = $5, position = $6"
        ))
        .bind(uuid(folder.id))
        .bind(uuid(folder.project_id))
        .bind(folder.parent.map(uuid))
        .bind(&folder.name)
        .bind(folder.summary.as_deref())
        .bind(folder.position)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn remove_folder(&mut self, id: FolderId) -> StoreResult<()> {
        // Sub-folders and the placements inside them go with it, by cascade.
        sqlx::query("DELETE FROM folders WHERE id = $1")
            .bind(uuid(id))
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    // -- placements -----------------------------------------------------------

    async fn placement(&mut self, endpoint: EndpointId) -> StoreResult<Option<PlacementRow>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            placement_columns!(),
            " FROM placements WHERE endpoint_id = $1"
        ))
        .bind(uuid(endpoint))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(placement_row).transpose()
    }

    async fn placements(&mut self, project: ProjectId) -> StoreResult<Vec<PlacementRow>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            placement_columns!(),
            " FROM placements WHERE project_id = $1"
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(placement_row).collect()
    }

    async fn put_placement(&mut self, placement: &PlacementRow) -> StoreResult<()> {
        sqlx::query(concat!(
            "INSERT INTO placements (",
            placement_columns!(),
            ") VALUES ($1,$2,$3,$4,$5)",
            " ON CONFLICT (endpoint_id) DO UPDATE SET project_id = $2,",
            " folder_id = $3, author = $4, at = $5"
        ))
        .bind(uuid(placement.endpoint))
        .bind(uuid(placement.project_id))
        .bind(uuid(placement.folder))
        .bind(json(&placement.author)?)
        .bind(placement.at)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn remove_placement(&mut self, endpoint: EndpointId) -> StoreResult<()> {
        sqlx::query("DELETE FROM placements WHERE endpoint_id = $1")
            .bind(uuid(endpoint))
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    // -- notes ----------------------------------------------------------------

    async fn endpoint_note(&mut self, endpoint: EndpointId) -> StoreResult<Option<EndpointNote>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            endpoint_note_columns!(),
            " FROM endpoint_notes WHERE endpoint_id = $1"
        ))
        .bind(uuid(endpoint))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(endpoint_note_row).transpose()
    }

    async fn endpoint_notes(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointNote>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            endpoint_note_columns!(),
            " FROM endpoint_notes WHERE project_id = $1"
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(endpoint_note_row).collect()
    }

    async fn put_endpoint_note(
        &mut self,
        project: ProjectId,
        note: &EndpointNote,
    ) -> StoreResult<()> {
        sqlx::query(concat!(
            "INSERT INTO endpoint_notes (endpoint_id, project_id, name, purpose, author, at)",
            " VALUES ($1,$2,$3,$4,$5,$6)",
            " ON CONFLICT (endpoint_id) DO UPDATE SET name = $3, purpose = $4,",
            " author = $5, at = $6"
        ))
        .bind(uuid(note.endpoint))
        .bind(uuid(project))
        .bind(note.name.as_deref())
        .bind(note.purpose.as_deref())
        .bind(json(&note.author)?)
        .bind(note.at)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn field_note(
        &mut self,
        endpoint: EndpointId,
        path: &str,
    ) -> StoreResult<Option<FieldNote>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            field_note_columns!(),
            " FROM field_notes WHERE endpoint_id = $1 AND path = $2"
        ))
        .bind(uuid(endpoint))
        .bind(path)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(field_note_row).transpose()
    }

    async fn field_notes(&mut self, endpoint: EndpointId) -> StoreResult<Vec<FieldNote>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            field_note_columns!(),
            " FROM field_notes WHERE endpoint_id = $1 ORDER BY path"
        ))
        .bind(uuid(endpoint))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(field_note_row).collect()
    }

    async fn put_field_note(&mut self, project: ProjectId, note: &FieldNote) -> StoreResult<()> {
        sqlx::query(concat!(
            "INSERT INTO field_notes",
            " (endpoint_id, path, project_id, text, values_seen, author, at)",
            " VALUES ($1,$2,$3,$4,$5,$6,$7)",
            " ON CONFLICT (endpoint_id, path) DO UPDATE SET text = $4,",
            " values_seen = $5, author = $6, at = $7"
        ))
        .bind(uuid(note.endpoint))
        .bind(&note.path)
        .bind(uuid(project))
        .bind(&note.text)
        .bind(json(&note.values)?)
        .bind(json(&note.author)?)
        .bind(note.at)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn remove_field_note(&mut self, endpoint: EndpointId, path: &str) -> StoreResult<()> {
        sqlx::query("DELETE FROM field_notes WHERE endpoint_id = $1 AND path = $2")
            .bind(uuid(endpoint))
            .bind(path)
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    // -- links ----------------------------------------------------------------

    async fn link(&mut self, id: LinkId) -> StoreResult<Option<Link>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            link_columns!(),
            " FROM links WHERE id = $1"
        ))
        .bind(uuid(id))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(link_row).transpose()
    }

    async fn find_link(&mut self, identity: &LinkIdentity) -> StoreResult<Option<Link>> {
        // Matches the links_identity index: NULL fields compare as ''.
        let row = sqlx::query(concat!(
            "SELECT ",
            link_columns!(),
            " FROM links WHERE from_endpoint = $1 AND to_endpoint = $2",
            " AND relation = $3 AND coalesce(from_field, '') = $4",
            " AND coalesce(to_field, '') = $5"
        ))
        .bind(uuid(identity.from))
        .bind(uuid(identity.to))
        .bind(text(identity.relation))
        .bind(identity.from_field.as_deref().unwrap_or(""))
        .bind(identity.to_field.as_deref().unwrap_or(""))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(link_row).transpose()
    }

    async fn links(&mut self, project: ProjectId) -> StoreResult<Vec<Link>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            link_columns!(),
            " FROM links WHERE project_id = $1"
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(link_row).collect()
    }

    async fn links_of(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Link>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            link_columns!(),
            " FROM links WHERE from_endpoint = $1 OR to_endpoint = $1"
        ))
        .bind(uuid(endpoint))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(link_row).collect()
    }

    async fn put_link(&mut self, project: ProjectId, link: &Link) -> StoreResult<()> {
        // Conflict on identity, not on id: core mints an id before it knows
        // whether an equal link already exists.
        sqlx::query(concat!(
            "INSERT INTO links (project_id, ",
            link_columns!(),
            ") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            " ON CONFLICT (from_endpoint, to_endpoint, relation,",
            " coalesce(from_field, ''), coalesce(to_field, ''))",
            " DO UPDATE SET evidence = $8, author = $9, at = $10"
        ))
        .bind(uuid(project))
        .bind(uuid(link.id))
        .bind(uuid(link.from))
        .bind(link.from_field.as_deref())
        .bind(uuid(link.to))
        .bind(link.to_field.as_deref())
        .bind(text(link.relation))
        .bind(&link.evidence)
        .bind(json(&link.author)?)
        .bind(link.at)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn remove_link(&mut self, id: LinkId) -> StoreResult<()> {
        sqlx::query("DELETE FROM links WHERE id = $1")
            .bind(uuid(id))
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    async fn restore(
        &mut self,
        project: ProjectId,
        target: &Target,
        before: Option<&Value>,
    ) -> StoreResult<()> {
        match (target, before) {
            // Nothing was there before the write, so undoing it removes the row.
            (Target::Folder { id }, None) => self.remove_folder(*id).await,
            (Target::Placement { endpoint }, None) => self.remove_placement(*endpoint).await,
            (Target::EndpointNote { endpoint }, None) => {
                sqlx::query("DELETE FROM endpoint_notes WHERE endpoint_id = $1")
                    .bind(uuid(*endpoint))
                    .execute(&mut *self.tx)
                    .await
                    .map(|_| ())
                    .map_err(unavailable)
            }
            (Target::FieldNote { endpoint, path }, None) => {
                self.remove_field_note(*endpoint, path).await
            }
            (Target::Link { id }, None) => self.remove_link(*id).await,

            (Target::Folder { id }, Some(before)) => {
                let kept: KeptFolder =
                    serde_json::from_value(before.clone()).map_err(unavailable)?;
                self.put_folder(&FolderRow {
                    id: *id,
                    project_id: project,
                    parent: kept.parent,
                    name: kept.name,
                    summary: kept.summary,
                    position: kept.position,
                })
                .await
            }
            (Target::Placement { endpoint }, Some(before)) => {
                let kept: KeptPlacement =
                    serde_json::from_value(before.clone()).map_err(unavailable)?;
                self.put_placement(&PlacementRow {
                    endpoint: *endpoint,
                    project_id: project,
                    folder: kept.folder,
                    author: kept.author,
                    at: kept.at,
                })
                .await
            }
            (Target::EndpointNote { endpoint }, Some(before)) => {
                let kept: KeptEndpointNote =
                    serde_json::from_value(before.clone()).map_err(unavailable)?;
                self.put_endpoint_note(
                    project,
                    &EndpointNote {
                        endpoint: *endpoint,
                        name: kept.name,
                        purpose: kept.purpose,
                        author: kept.author,
                        at: kept.at,
                    },
                )
                .await
            }
            (Target::FieldNote { endpoint, path }, Some(before)) => {
                let kept: KeptFieldNote =
                    serde_json::from_value(before.clone()).map_err(unavailable)?;
                self.put_field_note(
                    project,
                    &FieldNote {
                        endpoint: *endpoint,
                        path: path.clone(),
                        text: kept.text,
                        values: kept.values,
                        author: kept.author,
                        at: kept.at,
                    },
                )
                .await
            }
            (Target::Link { id }, Some(before)) => {
                let kept: KeptLink = serde_json::from_value(before.clone()).map_err(unavailable)?;
                self.put_link(
                    project,
                    &Link {
                        id: *id,
                        from: kept.from,
                        from_field: kept.from_field,
                        to: kept.to,
                        to_field: kept.to_field,
                        relation: kept.relation,
                        evidence: kept.evidence,
                        author: kept.author,
                        at: kept.at,
                    },
                )
                .await
            }
        }
    }
}

// What a revision keeps for each target. Core writes these shapes in
// `apply.rs`; reading them back is the store's half of the same agreement.

#[derive(serde::Deserialize)]
struct KeptFolder {
    parent: Option<FolderId>,
    name: String,
    summary: Option<String>,
    position: i32,
}

#[derive(serde::Deserialize)]
struct KeptPlacement {
    folder: FolderId,
    author: Author,
    at: DateTime<Utc>,
}

#[derive(serde::Deserialize)]
struct KeptEndpointNote {
    name: Option<String>,
    purpose: Option<String>,
    author: Author,
    at: DateTime<Utc>,
}

#[derive(serde::Deserialize)]
struct KeptFieldNote {
    text: String,
    values: Vec<String>,
    author: Author,
    at: DateTime<Utc>,
}

#[derive(serde::Deserialize)]
struct KeptLink {
    from: EndpointId,
    from_field: Option<String>,
    to: EndpointId,
    to_field: Option<String>,
    relation: Relation,
    evidence: String,
    author: Author,
    at: DateTime<Utc>,
}

fn link_row(row: &PgRow) -> StoreResult<Link> {
    Ok(Link {
        id: id(row.get("id")),
        from: id(row.get("from_endpoint")),
        from_field: row.get("from_field"),
        to: id(row.get("to_endpoint")),
        to_field: row.get("to_field"),
        relation: parse(row.get("relation"))?,
        evidence: row.get("evidence"),
        author: author(row.get("author"))?,
        at: row.get("at"),
    })
}

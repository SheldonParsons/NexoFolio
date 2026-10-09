//! Revisions SQL, all executed on the caller's existing transaction.
use super::{
    codec::{json, unavailable, uuid},
    transaction::PostgresTx,
};
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, FolderId, ProjectId, RoundId};
use nexofolio_contracts::knowledge::{
    Author, EndpointNote, FieldNote, Link, Relation, Revision, Target,
};
use nexofolio_knowledge_contracts::StoreResult;
use nexofolio_knowledge_contracts::{FolderRow, KnowledgeTx, PlacementRow};
use serde_json::Value;
use sqlx::Row;

impl PostgresTx {
    pub(super) async fn db_record_revision(&mut self, revision: &Revision) -> StoreResult<()> {
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

    pub(super) async fn db_revisions(&mut self, round: RoundId) -> StoreResult<Vec<Revision>> {
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

    pub(super) async fn db_restore(
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
                    author: kept.author,
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
                        context: kept.context,
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
                        context: kept.context,
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
                        context: kept.context,
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
    #[serde(default)]
    author: Option<Author>,
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
    #[serde(default)]
    context: nexofolio_contracts::knowledge::EndpointContext,
    author: Author,
    at: DateTime<Utc>,
}

#[derive(serde::Deserialize)]
struct KeptFieldNote {
    text: String,
    values: Vec<String>,
    #[serde(default)]
    context: nexofolio_contracts::knowledge::FieldContext,
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
    #[serde(default)]
    context: nexofolio_contracts::knowledge::LinkContext,
    author: Author,
    at: DateTime<Utc>,
}

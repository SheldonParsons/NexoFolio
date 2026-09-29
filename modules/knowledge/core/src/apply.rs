//! Turning one [`Command`] into store writes, and remembering what it replaced.
//!
//! Every write records the previous value first, so a round can be undone by
//! replaying its revisions newest first. `before: None` means the write created
//! something, and undoing it removes the row.
//!
//! This is the only place that knows what a command means. A new curate pass
//! produces commands and touches nothing here.

use chrono::{DateTime, Utc};
use nexofolio_common::{LinkId, ProjectId, RoundId};
use nexofolio_contracts::knowledge::{
    Author, Command, EndpointNote, FieldNote, KnowledgeError, KnowledgeResult, Link, Revision,
    Target,
};
use nexofolio_knowledge_contracts::{FolderRow, KnowledgeTx, LinkIdentity, PlacementRow};
use serde_json::{Value, json};

use crate::store_error;

/// Applies one command, recording its revision first.
///
/// `seq` orders writes inside the round. Rollback replays them in reverse, so
/// two commands touching the same row undo in the right order.
pub async fn one(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    round: RoundId,
    seq: i64,
    author: &Author,
    at: DateTime<Utc>,
    command: &Command,
) -> KnowledgeResult<()> {
    match command {
        Command::PutFolder {
            id,
            parent,
            name,
            summary,
            position,
        } => {
            let folders = tx.folders(project).await.map_err(store_error)?;
            crate::tree::check_move(*id, *parent, &folders)?;
            let before = folders.iter().find(|f| f.id == *id).map(|f| {
                json!({
                    "parent": f.parent,
                    "name": f.name,
                    "summary": f.summary,
                    "position": f.position,
                })
            });
            record(tx, round, seq, Target::Folder { id: *id }, before).await?;
            tx.put_folder(&FolderRow {
                id: *id,
                project_id: project,
                parent: *parent,
                name: name.clone(),
                summary: summary.clone(),
                position: *position,
            })
            .await
            .map_err(store_error)
        }

        Command::RemoveFolder { id } => {
            let Some(folder) = tx.folder(*id).await.map_err(store_error)? else {
                return Err(KnowledgeError::NoSuchFolder);
            };
            let before = json!({
                "parent": folder.parent,
                "name": folder.name,
                "summary": folder.summary,
                "position": folder.position,
            });
            record(tx, round, seq, Target::Folder { id: *id }, Some(before)).await?;
            tx.remove_folder(*id).await.map_err(store_error)
        }

        Command::Place { endpoint, folder } => {
            if tx.folder(*folder).await.map_err(store_error)?.is_none() {
                return Err(KnowledgeError::NoSuchFolder);
            }
            let before = tx
                .placement(*endpoint)
                .await
                .map_err(store_error)?
                .map(|p| json!({ "folder": p.folder, "author": p.author, "at": p.at }));
            record(
                tx,
                round,
                seq,
                Target::Placement {
                    endpoint: *endpoint,
                },
                before,
            )
            .await?;
            tx.put_placement(&PlacementRow {
                endpoint: *endpoint,
                project_id: project,
                folder: *folder,
                author: author.clone(),
                at,
            })
            .await
            .map_err(store_error)
        }

        Command::Unplace { endpoint } => {
            let before = tx
                .placement(*endpoint)
                .await
                .map_err(store_error)?
                .map(|p| json!({ "folder": p.folder, "author": p.author, "at": p.at }));
            record(
                tx,
                round,
                seq,
                Target::Placement {
                    endpoint: *endpoint,
                },
                before,
            )
            .await?;
            tx.remove_placement(*endpoint).await.map_err(store_error)
        }

        Command::PutEndpointNote {
            endpoint,
            name,
            purpose,
        } => {
            let existing = tx.endpoint_note(*endpoint).await.map_err(store_error)?;
            let before = existing.as_ref().map(
                |n| json!({ "name": n.name, "purpose": n.purpose, "author": n.author, "at": n.at }),
            );
            record(
                tx,
                round,
                seq,
                Target::EndpointNote {
                    endpoint: *endpoint,
                },
                before,
            )
            .await?;
            // `None` leaves the old value alone, so the describe pass can write
            // a name without clearing a purpose someone edited by hand.
            let note = EndpointNote {
                endpoint: *endpoint,
                name: name
                    .clone()
                    .or_else(|| existing.as_ref().and_then(|n| n.name.clone())),
                purpose: purpose
                    .clone()
                    .or_else(|| existing.as_ref().and_then(|n| n.purpose.clone())),
                author: author.clone(),
                at,
            };
            tx.put_endpoint_note(project, &note)
                .await
                .map_err(store_error)
        }

        Command::PutFieldNote {
            endpoint,
            path,
            text,
            values,
        } => {
            let before = tx
                .field_note(*endpoint, path)
                .await
                .map_err(store_error)?
                .map(|n| json!({ "text": n.text, "values": n.values, "author": n.author, "at": n.at }));
            record(
                tx,
                round,
                seq,
                Target::FieldNote {
                    endpoint: *endpoint,
                    path: path.clone(),
                },
                before,
            )
            .await?;
            tx.put_field_note(
                project,
                &FieldNote {
                    endpoint: *endpoint,
                    path: path.clone(),
                    text: text.clone(),
                    values: values.clone(),
                    author: author.clone(),
                    at,
                },
            )
            .await
            .map_err(store_error)
        }

        Command::RemoveFieldNote { endpoint, path } => {
            let before = tx
                .field_note(*endpoint, path)
                .await
                .map_err(store_error)?
                .map(|n| json!({ "text": n.text, "values": n.values, "author": n.author, "at": n.at }));
            record(
                tx,
                round,
                seq,
                Target::FieldNote {
                    endpoint: *endpoint,
                    path: path.clone(),
                },
                before,
            )
            .await?;
            tx.remove_field_note(*endpoint, path)
                .await
                .map_err(store_error)
        }

        Command::PutLink {
            from,
            from_field,
            to,
            to_field,
            relation,
            evidence,
        } => {
            let identity = LinkIdentity {
                from: *from,
                from_field: from_field.clone(),
                to: *to,
                to_field: to_field.clone(),
                relation: *relation,
            };
            // Rediscovering a link refreshes it rather than adding a duplicate.
            let existing = tx.find_link(&identity).await.map_err(store_error)?;
            let id = existing.as_ref().map(|l| l.id).unwrap_or_else(LinkId::new);
            // The whole link, not just what changes: a rollback of this and a
            // rollback of a removal then restore the same way.
            let before = existing.as_ref().map(|l| {
                json!({
                    "from": l.from, "from_field": l.from_field,
                    "to": l.to, "to_field": l.to_field,
                    "relation": l.relation, "evidence": l.evidence,
                    "author": l.author, "at": l.at,
                })
            });
            record(tx, round, seq, Target::Link { id }, before).await?;
            tx.put_link(
                project,
                &Link {
                    id,
                    from: *from,
                    from_field: from_field.clone(),
                    to: *to,
                    to_field: to_field.clone(),
                    relation: *relation,
                    evidence: evidence.clone(),
                    author: author.clone(),
                    at,
                },
            )
            .await
            .map_err(store_error)
        }

        Command::RemoveLink { id } => {
            let before = tx.link(*id).await.map_err(store_error)?.map(|l| {
                json!({
                    "from": l.from, "from_field": l.from_field,
                    "to": l.to, "to_field": l.to_field,
                    "relation": l.relation, "evidence": l.evidence,
                    "author": l.author, "at": l.at,
                })
            });
            record(tx, round, seq, Target::Link { id: *id }, before).await?;
            tx.remove_link(*id).await.map_err(store_error)
        }
    }
}

async fn record(
    tx: &mut dyn KnowledgeTx,
    round: RoundId,
    seq: i64,
    target: Target,
    before: Option<Value>,
) -> KnowledgeResult<()> {
    tx.record_revision(&Revision {
        round,
        seq,
        target,
        before,
    })
    .await
    .map_err(store_error)
}

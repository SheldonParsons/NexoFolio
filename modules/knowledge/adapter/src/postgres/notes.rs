//! Notes SQL, all executed on the caller's existing transaction.
use super::{
    codec::{author, id, json, unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::knowledge::{EndpointNote, FieldNote};
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::{Row, postgres::PgRow};

macro_rules! endpoint_note_columns {
    () => {
        "endpoint_id, name, purpose, context, author, at"
    };
}

fn endpoint_note_row(row: &PgRow) -> StoreResult<EndpointNote> {
    Ok(EndpointNote {
        endpoint: id(row.get("endpoint_id")),
        name: row.get("name"),
        purpose: row.get("purpose"),
        context: serde_json::from_value(row.get("context")).map_err(unavailable)?,
        author: author(row.get("author"))?,
        at: row.get("at"),
    })
}

macro_rules! field_note_columns {
    () => {
        "endpoint_id, path, text, values_seen, context, author, at"
    };
}

fn field_note_row(row: &PgRow) -> StoreResult<FieldNote> {
    Ok(FieldNote {
        endpoint: id(row.get("endpoint_id")),
        path: row.get("path"),
        text: row.get("text"),
        values: serde_json::from_value(row.get("values_seen")).map_err(unavailable)?,
        context: serde_json::from_value(row.get("context")).map_err(unavailable)?,
        author: author(row.get("author"))?,
        at: row.get("at"),
    })
}

impl PostgresTx {
    pub(super) async fn db_project_field_notes(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Vec<FieldNote>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            field_note_columns!(),
            " FROM field_notes WHERE project_id = $1 ORDER BY endpoint_id, path"
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(field_note_row).collect()
    }
    pub(super) async fn db_endpoint_note(
        &mut self,
        endpoint: EndpointId,
    ) -> StoreResult<Option<EndpointNote>> {
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

    pub(super) async fn db_endpoint_notes(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Vec<EndpointNote>> {
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

    pub(super) async fn db_put_endpoint_note(
        &mut self,
        project: ProjectId,
        note: &EndpointNote,
    ) -> StoreResult<()> {
        sqlx::query(concat!(
            "INSERT INTO endpoint_notes (endpoint_id, project_id, name, purpose, author, at, context)",
            " VALUES ($1,$2,$3,$4,$5,$6,$7)",
            " ON CONFLICT (endpoint_id) DO UPDATE SET name = $3, purpose = $4,",
            " author = $5, at = $6, context = $7"
        ))
        .bind(uuid(note.endpoint))
        .bind(uuid(project))
        .bind(note.name.as_deref())
        .bind(note.purpose.as_deref())
        .bind(json(&note.author)?)
        .bind(note.at)
        .bind(json(&note.context)?)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    pub(super) async fn db_field_note(
        &mut self,
        endpoint: EndpointId,
        path: &str,
    ) -> StoreResult<Option<FieldNote>> {
        if let Some(buffer) = &self.apply_buffer {
            if buffer.owned.contains(&endpoint) {
                return Ok(buffer.fields.get(&(endpoint, path.to_owned())).cloned());
            }
        }
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

    pub(super) async fn db_field_notes(
        &mut self,
        endpoint: EndpointId,
    ) -> StoreResult<Vec<FieldNote>> {
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

    pub(super) async fn db_put_field_note(
        &mut self,
        project: ProjectId,
        note: &FieldNote,
    ) -> StoreResult<()> {
        if let Some(buffer) = &mut self.apply_buffer {
            if buffer.project != project || !buffer.owned.contains(&note.endpoint) {
                return Err(nexofolio_knowledge_contracts::StoreError::Unavailable);
            }
            let key = (note.endpoint, note.path.clone());
            buffer.fields.insert(key.clone(), note.clone());
            buffer.pending.insert(key, Some(note.clone()));
            if buffer.revisions.len() >= 100 {
                self.flush_apply().await?;
            }
            return Ok(());
        }
        sqlx::query(concat!(
            "INSERT INTO field_notes",
            " (endpoint_id, path, project_id, text, values_seen, author, at, context)",
            " VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            " ON CONFLICT (endpoint_id, path) DO UPDATE SET text = $4,",
            " values_seen = $5, author = $6, at = $7, context = $8"
        ))
        .bind(uuid(note.endpoint))
        .bind(&note.path)
        .bind(uuid(project))
        .bind(&note.text)
        .bind(json(&note.values)?)
        .bind(json(&note.author)?)
        .bind(note.at)
        .bind(json(&note.context)?)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    pub(super) async fn db_remove_field_note(
        &mut self,
        endpoint: EndpointId,
        path: &str,
    ) -> StoreResult<()> {
        if let Some(buffer) = &mut self.apply_buffer {
            let key = (endpoint, path.to_owned());
            buffer.fields.remove(&key);
            buffer.pending.insert(key, None);
            if buffer.revisions.len() >= 100 {
                self.flush_apply().await?;
            }
            return Ok(());
        }
        sqlx::query("DELETE FROM field_notes WHERE endpoint_id = $1 AND path = $2")
            .bind(uuid(endpoint))
            .bind(path)
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }
}

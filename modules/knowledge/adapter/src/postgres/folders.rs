//! Folders SQL, all executed on the caller's existing transaction.
use super::{
    codec::{author, id, json, unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::{FolderId, ProjectId};
use nexofolio_knowledge_contracts::FolderRow;
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

macro_rules! folder_columns {
    () => {
        "id, project_id, parent_id, name, summary, position, author"
    };
}

fn folder_row(row: &PgRow) -> StoreResult<FolderRow> {
    Ok(FolderRow {
        id: id(row.get("id")),
        author: row
            .get::<Option<serde_json::Value>, _>("author")
            .map(author)
            .transpose()?,
        project_id: id(row.get("project_id")),
        parent: row.get::<Option<Uuid>, _>("parent_id").map(id),
        name: row.get("name"),
        summary: row.get("summary"),
        position: row.get("position"),
    })
}

impl PostgresTx {
    pub(super) async fn db_folder(&mut self, id: FolderId) -> StoreResult<Option<FolderRow>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            folder_columns!(),
            " FROM folders WHERE id = $1"
        ))
        .bind(uuid(id))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(folder_row).transpose()
    }

    pub(super) async fn db_folders(&mut self, project: ProjectId) -> StoreResult<Vec<FolderRow>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            folder_columns!(),
            " FROM folders WHERE project_id = $1"
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(folder_row).collect()
    }

    pub(super) async fn db_put_folder(&mut self, folder: &FolderRow) -> StoreResult<()> {
        sqlx::query(concat!(
            "INSERT INTO folders (",
            folder_columns!(),
            ") VALUES ($1,$2,$3,$4,$5,$6,$7)",
            " ON CONFLICT (id) DO UPDATE SET parent_id = $3, name = $4,",
            " summary = $5, position = $6, author = $7"
        ))
        .bind(uuid(folder.id))
        .bind(uuid(folder.project_id))
        .bind(folder.parent.map(uuid))
        .bind(&folder.name)
        .bind(folder.summary.as_deref())
        .bind(folder.position)
        .bind(folder.author.as_ref().map(json).transpose()?)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    pub(super) async fn db_remove_folder(&mut self, id: FolderId) -> StoreResult<()> {
        // Sub-folders and the placements inside them go with it, by cascade.
        sqlx::query("DELETE FROM folders WHERE id = $1")
            .bind(uuid(id))
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }
}

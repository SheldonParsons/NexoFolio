//! Placements SQL, all executed on the caller's existing transaction.
use super::{
    codec::{author, id, json, unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_knowledge_contracts::PlacementRow;
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::{Row, postgres::PgRow};

macro_rules! placement_columns {
    () => {
        "endpoint_id, project_id, folder_id, author, at"
    };
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

impl PostgresTx {
    pub(super) async fn db_placement(
        &mut self,
        endpoint: EndpointId,
    ) -> StoreResult<Option<PlacementRow>> {
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

    pub(super) async fn db_placements(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Vec<PlacementRow>> {
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

    pub(super) async fn db_put_placement(&mut self, placement: &PlacementRow) -> StoreResult<()> {
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

    pub(super) async fn db_remove_placement(&mut self, endpoint: EndpointId) -> StoreResult<()> {
        sqlx::query("DELETE FROM placements WHERE endpoint_id = $1")
            .bind(uuid(endpoint))
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }
}

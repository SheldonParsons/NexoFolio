use super::{
    codec::{unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::field_values::FieldValueArchive;
use nexofolio_knowledge_contracts::StoreResult;
impl PostgresTx {
    pub(super) async fn db_value_archive(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> StoreResult<Option<FieldValueArchive>> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT archive FROM field_value_archives WHERE project_id=$1 AND candidate_id=$2",
        )
        .bind(uuid(project))
        .bind(candidate)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        value
            .map(|v| serde_json::from_str(&v).map_err(unavailable))
            .transpose()
    }
    pub(super) async fn db_put_value_archive(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        archive: &FieldValueArchive,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO field_value_archives(project_id,candidate_id,archive) VALUES($1,$2,$3)",
        )
        .bind(uuid(project))
        .bind(candidate)
        .bind(serde_json::to_string(archive).map_err(unavailable)?)
        .execute(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        Ok(())
    }
}

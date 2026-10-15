use super::{
    codec::{unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::field_values::FieldValueArchive;
use nexofolio_knowledge_contracts::StoreResult;
impl PostgresTx {
    pub(super) async fn db_value_archive_field(
        &mut self,
        q: &nexofolio_contracts::field_values::FieldValueQuery,
        value_id: Option<&str>,
    ) -> StoreResult<Option<FieldValueArchive>> {
        // JSON keeps numeric tokens. Legacy NUL-bearing text must use the original
        // decoder: PostgreSQL JSON extraction also rejects NUL. V3 chunks avoid this fallback.
        let value: Option<String> = sqlx::query_scalar(
            r#"
WITH source AS MATERIALIZED (
  SELECT archive AS raw, CASE WHEN strpos(archive, chr(92)||'u0000')>0 THEN NULL ELSE archive::json END AS a
  FROM field_value_archives WHERE project_id=$1 AND candidate_id=$2
), field AS (
  SELECT a->'fields'->$3 AS f FROM source
), needed AS (
  SELECT DISTINCT p->>'document' AS id FROM field,
       LATERAL json_array_elements(f->'values') v,
       LATERAL json_array_elements(v->'occurrences') p
  WHERE $4::text IS NULL OR v->>'id'=$4
)
SELECT CASE WHEN a IS NULL THEN raw ELSE json_build_object('version',a->'version',
  'fields',CASE WHEN f IS NULL THEN '{}'::json ELSE json_build_object($3::text,f) END,
  'documents',coalesce((SELECT json_object_agg(d.key,d.value) FROM json_each(a->'documents') d
                       WHERE d.key IN (SELECT id FROM needed)), '{}'::json))::text END
FROM source CROSS JOIN field
"#,
        )
        .bind(uuid(q.project))
        .bind(q.archive)
        .bind(format!("{}/{}", q.endpoint, q.field))
        .bind(value_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        value
            .map(|v| serde_json::from_str(&v).map_err(unavailable))
            .transpose()
    }

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

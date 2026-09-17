use super::*;
use nexofolio_contracts::{ObservationAssessment, ObservationAssessmentPage};
use nexofolio_knowledge::AssessmentReader;

pub(crate) const MATERIAL: &str = r#"SELECT jsonb_build_object(
 'project_id',i.project_id,'interface_id',o.interface_id,'environment_id',o.environment_id,
 'ingestion_id',o.ingestion_id,'base_revision_id',CASE WHEN o.outcome='created' THEN NULL ELSE o.compared_revision_id END,
 'extractor_version',coalesce(o.extractor_version,f.proposed_definition->>'extractor_version',CASE WHEN o.outcome='created' THEN r.definition->>'extractor_version' END),
 'assessment',o.assessment,'incoming_definition',CASE WHEN o.outcome='created' THEN r.definition ELSE f.proposed_definition END) AS value
 FROM interface_observations o JOIN ingestion_inbox i ON i.id=o.ingestion_id
 JOIN interface_observed_revisions r ON r.id=o.compared_revision_id
 LEFT JOIN interface_observed_differences f ON f.id=o.difference_id
 WHERE i.project_id=$1"#;
fn decode(value: Value) -> Result<ObservationAssessment> {
    serde_json::from_value(value).map_err(|_| Error::Unavailable {
        component: "observation_assessment",
    })
}
#[async_trait]
impl AssessmentReader for PostgresDocuments {
    async fn assessment(
        &self,
        user: UserId,
        project: ProjectId,
        ingestion_id: Uuid,
    ) -> Result<ObservationAssessment> {
        self.authorize(user, project).await?;
        let query = format!("{MATERIAL} AND o.ingestion_id=$2");
        let value = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .bind(uuid(project))
            .bind(ingestion_id)
            .fetch_optional(&self.database.pool)
            .await
            .map_err(db_error)?
            .ok_or(Error::NotFound)?;
        decode(value)
    }
    async fn assessments(
        &self,
        user: UserId,
        project: ProjectId,
        interface: InterfaceId,
        q: DocumentQuery,
    ) -> Result<ObservationAssessmentPage> {
        pagination(q.page, q.limit)?;
        if q.query.is_some() {
            return Err(Error::invalid("assessment query filter is unsupported"));
        }
        self.detail(user, project, q.environment_id, interface)
            .await?;
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let total=sqlx::query_scalar("SELECT count(*) FROM interface_observations WHERE interface_id=$1 AND environment_id=$2").bind(uuid(interface)).bind(uuid(q.environment_id)).fetch_one(&mut *tx).await.map_err(db_error)?;
        let query = format!(
            "{MATERIAL} AND o.interface_id=$2 AND o.environment_id=$3 ORDER BY o.processed_at,o.ingestion_id LIMIT $4 OFFSET $5"
        );
        let values: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .bind(uuid(project))
            .bind(uuid(interface))
            .bind(uuid(q.environment_id))
            .bind(i64::from(q.limit))
            .bind(i64::from((q.page - 1) * q.limit))
            .fetch_all(&mut *tx)
            .await
            .map_err(db_error)?;
        let items = values.into_iter().map(decode).collect::<Result<Vec<_>>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(ObservationAssessmentPage {
            items,
            page: q.page,
            limit: q.limit,
            total,
        })
    }
}

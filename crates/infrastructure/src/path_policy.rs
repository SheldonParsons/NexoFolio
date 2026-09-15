use crate::Postgres;
use nexofolio_contracts::{Error, PathPolicy, ProjectId, Result};
use serde_json::Value;
use uuid::Uuid;
pub struct PostgresPathPolicies {
    database: Postgres,
}
impl PostgresPathPolicies {
    pub fn new(database: Postgres) -> Self {
        Self { database }
    }
}
#[async_trait::async_trait]
impl nexofolio_intake::ProjectPathPolicies for PostgresPathPolicies {
    async fn read(&self, project: ProjectId) -> Result<PathPolicy> {
        let value: Value = sqlx::query_scalar("SELECT path_policy FROM projects WHERE id=$1")
            .bind(project.to_string().parse::<Uuid>().expect("UUID"))
            .fetch_optional(&self.database.pool)
            .await
            .map_err(|_| Error::Unavailable {
                component: "path_policy",
            })?
            .ok_or(Error::NotFound)?;
        serde_json::from_value(value).map_err(|_| Error::Unavailable {
            component: "path_policy",
        })
    }
    async fn set(&self, project: ProjectId, policy: &PathPolicy) -> Result<()> {
        if !policy.valid() {
            return Err(Error::invalid("invalid path policy"));
        }
        let result = sqlx::query("UPDATE projects SET path_policy=$2 WHERE id=$1")
            .bind(project.to_string().parse::<Uuid>().expect("UUID"))
            .bind(serde_json::to_value(policy).expect("serializes"))
            .execute(&self.database.pool)
            .await
            .map_err(|_| Error::Unavailable {
                component: "path_policy",
            })?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
        Ok(())
    }
}

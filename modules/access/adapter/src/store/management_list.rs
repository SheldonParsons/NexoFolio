//! Paginated management read model, scoped to the authenticated instance.
use super::{PostgresAccess, db_error, management::require_manager, typed};
use nexofolio_access_contracts::{
    ManagementProject, ManagementProjects, PermissionSource, SessionPrincipal,
};
use nexofolio_common::{Error, Result};
use sqlx::Row;
use std::collections::HashMap;

impl PostgresAccess {
    pub async fn management_projects(
        &self,
        principal: &SessionPrincipal,
        page: u32,
        limit: u32,
        query: &str,
        origin: Option<&str>,
    ) -> Result<ManagementProjects> {
        if page == 0
            || page > 100000
            || !(1..=100).contains(&limit)
            || query.chars().count() > 100
            || origin.is_some_and(|value| !matches!(value, "local" | "zentao"))
        {
            return Err(Error::invalid("invalid management query"));
        }
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        require_manager(&mut tx, principal).await?;
        // Literal substring matching: '%' and '_' supplied by the user are not SQL wildcards.
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM projects WHERE instance=$1 AND strpos(lower(name),lower($2))>0 AND ($3::text IS NULL OR origin=$3)")
            .bind(&principal.instance).bind(query.trim()).bind(origin).fetch_one(&mut *tx).await.map_err(db_error)?;
        let rows = sqlx::query("SELECT id,name,status,origin FROM projects WHERE instance=$1 AND strpos(lower(name),lower($2))>0 AND ($3::text IS NULL OR origin=$3) ORDER BY name,id LIMIT $4 OFFSET $5")
            .bind(&principal.instance).bind(query.trim()).bind(origin).bind(i64::from(limit)).bind(i64::from(page-1)*i64::from(limit)).fetch_all(&mut *tx).await.map_err(db_error)?;
        let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.get("id")).collect();
        let bindings = sqlx::query("SELECT b.project_id,s.id,s.name,s.status FROM project_permission_sources b JOIN projects s ON s.id=b.source_project_id AND s.instance=b.instance WHERE b.instance=$1 AND b.project_id=ANY($2) ORDER BY s.name,s.id")
            .bind(&principal.instance).bind(&ids).fetch_all(&mut *tx).await.map_err(db_error)?;
        let mut sources: HashMap<uuid::Uuid, Vec<PermissionSource>> = HashMap::new();
        for row in bindings {
            sources
                .entry(row.get("project_id"))
                .or_default()
                .push(PermissionSource {
                    project_id: typed(row.get("id")),
                    name: row.get("name"),
                    status: row.get("status"),
                });
        }
        let items = rows
            .into_iter()
            .map(|r| {
                let id = r.get("id");
                ManagementProject {
                    project_id: typed(id),
                    name: r.get("name"),
                    status: r.get("status"),
                    origin: r.get("origin"),
                    permission_sources: sources.remove(&id).unwrap_or_default(),
                }
            })
            .collect();
        tx.commit().await.map_err(db_error)?;
        Ok(ManagementProjects {
            items,
            total: total as u64,
            page,
            limit,
        })
    }
}

use super::environments::{by_id, by_name};
use super::{PostgresAccess, typed, uuid};
use async_trait::async_trait;
use nexofolio_access_contracts::validate_environment_name;
use nexofolio_contracts::scope::{
    CollectTarget, EnvironmentSelector, ResolvedTarget, ScopeError, SiteBinding, SiteRegistry,
    SiteScope, TargetResolver,
};
use sqlx::{Postgres as Pg, Row, Transaction};
use uuid::Uuid;

#[async_trait]
impl TargetResolver for PostgresAccess {
    async fn resolve(&self, target: &CollectTarget) -> Result<ResolvedTarget, ScopeError> {
        let project = uuid(target.project_id);
        let mut tx = self.database.pool.begin().await.map_err(unavailable)?;
        require_project(&mut tx, project).await?;
        let environment = match &target.environment {
            None => None,
            Some(EnvironmentSelector::Id(id)) => Some(
                by_id(&mut tx, project, uuid(id))
                    .await
                    .map_err(unavailable)?
                    .ok_or(ScopeError::UnknownEnvironment)?,
            ),
            Some(EnvironmentSelector::Name(name)) => {
                validate_environment_name(name)
                    .map_err(|_| ScopeError::InvalidEnvironmentName(name.clone()))?;
                Some(by_name(&mut tx, project, name).await.map_err(unavailable)?)
            }
        };
        if let (Some(environment), Some(site)) = (&environment, &target.site) {
            sqlx::query("INSERT INTO environment_sites(environment_id,origin,prefix) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                .bind(uuid(environment.id))
                .bind(site.origin())
                .bind(site.prefix())
                .execute(&mut *tx)
                .await
                .map_err(unavailable)?;
        }
        tx.commit().await.map_err(unavailable)?;
        Ok(ResolvedTarget {
            project_id: target.project_id,
            environment_id: environment.map(|environment| environment.id),
            site: target.site.clone(),
            source_url: target.source_url.clone(),
        })
    }
}

#[async_trait]
impl SiteRegistry for PostgresAccess {
    async fn lookup(&self, page_url: &str) -> Result<Option<SiteBinding>, ScopeError> {
        let Some(origin) = SiteScope::origin_of(page_url) else {
            return Ok(None);
        };
        let rows = sqlx::query(
            "SELECT origin,prefix,project_id,environment_id FROM sites WHERE origin=$1",
        )
        .bind(origin)
        .fetch_all(&self.database.pool)
        .await
        .map_err(unavailable)?;
        let mut best: Option<SiteBinding> = None;
        for row in rows {
            let origin: String = row.try_get("origin").map_err(unavailable)?;
            let prefix: String = row.try_get("prefix").map_err(unavailable)?;
            let site = SiteScope::new(&origin, &prefix)?;
            if !site.contains(page_url)
                || best
                    .as_ref()
                    .is_some_and(|b| b.site.specificity() >= site.specificity())
            {
                continue;
            }
            best = Some(SiteBinding {
                site,
                project_id: typed(row.try_get("project_id").map_err(unavailable)?),
                environment_id: typed(row.try_get("environment_id").map_err(unavailable)?),
            });
        }
        Ok(best)
    }

    async fn bind(&self, binding: SiteBinding) -> Result<(), ScopeError> {
        let project = uuid(binding.project_id);
        let mut tx = self.database.pool.begin().await.map_err(unavailable)?;
        require_project(&mut tx, project).await?;
        by_id(&mut tx, project, uuid(binding.environment_id))
            .await
            .map_err(unavailable)?
            .ok_or(ScopeError::UnknownEnvironment)?;
        sqlx::query("INSERT INTO sites(origin,prefix,project_id,environment_id) VALUES($1,$2,$3,$4) ON CONFLICT(origin,prefix) DO UPDATE SET project_id=excluded.project_id, environment_id=excluded.environment_id, bound_at=now()")
            .bind(binding.site.origin())
            .bind(binding.site.prefix())
            .bind(project)
            .bind(uuid(binding.environment_id))
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
}

async fn require_project(tx: &mut Transaction<'_, Pg>, project: Uuid) -> Result<(), ScopeError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id=$1)")
        .bind(project)
        .fetch_one(&mut **tx)
        .await
        .map_err(unavailable)?;
    exists.then_some(()).ok_or(ScopeError::UnknownProject)
}

/// Collect callers only need to know whether retrying may help.
fn unavailable<E>(_: E) -> ScopeError {
    ScopeError::Unavailable
}

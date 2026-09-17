use crate::Postgres;
use async_trait::async_trait;
use nexofolio_application::CatalogActivationStore;
use nexofolio_contracts::*;
use nexofolio_knowledge::OfficialCatalogReader;
use serde_json::Value;
use sqlx::{Row, Transaction};
use uuid::Uuid;
#[derive(Clone)]
pub struct PostgresOfficialCatalog {
    database: Postgres,
}
impl PostgresOfficialCatalog {
    pub fn new(database: Postgres) -> Self {
        Self { database }
    }
}
fn uuid(id: impl ToString) -> Uuid {
    id.to_string().parse().expect("typed UUID")
}
fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "official_catalog",
    }
}
fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|_| Error::Unavailable {
        component: "official_catalog_contract",
    })
}
fn pagination(page: u32, limit: u32) -> Result<()> {
    if !(1..=100000).contains(&page) || !(1..=100).contains(&limit) {
        Err(Error::invalid("invalid pagination"))
    } else {
        Ok(())
    }
}
pub(crate) use crate::project_access::authorize_project as authorize;

struct State {
    system: Uuid,
    current: Option<Uuid>,
    generation: i64,
}
async fn state(tx: &mut Transaction<'_, sqlx::Postgres>, project: ProjectId) -> Result<State> {
    let sql = "SELECT unclassified_id,current_version_id,generation FROM project_catalogs WHERE project_id=$1";
    let r = sqlx::query(sql)
        .bind(uuid(project))
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_error)?
        .ok_or(Error::NotFound)?;
    Ok(State {
        system: r.get("unclassified_id"),
        current: r.get("current_version_id"),
        generation: r.get("generation"),
    })
}
impl PostgresOfficialCatalog {
    async fn read_tx(
        &self,
        user: UserId,
        project: ProjectId,
    ) -> Result<Transaction<'static, sqlx::Postgres>> {
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        authorize(&mut tx, user, project).await?;
        Ok(tx)
    }
}
#[async_trait]
impl OfficialCatalogReader for PostgresOfficialCatalog {
    async fn current(&self, user: UserId, project: ProjectId) -> Result<OfficialCatalog> {
        let mut tx = self.read_tx(user, project).await?;
        let s = state(&mut tx, project).await?;
        let version = sqlx::query(
            "SELECT source_task_id,maintenance_run_id,candidate FROM catalog_versions WHERE id=$1 AND project_id=$2",
        )
        .bind(s.current)
        .bind(uuid(project))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;
        let source_task_id = version
            .as_ref()
            .and_then(|r| r.get::<Option<Uuid>, _>("source_task_id"))
            .map(|v| v.to_string().parse().unwrap());
        let source_run_id = version
            .as_ref()
            .and_then(|r| r.get::<Option<Uuid>, _>("maintenance_run_id"));
        let candidate: Option<DirectoryCandidate> =
            version.map(|r| decode(r.get("candidate"))).transpose()?;
        let rows = sqlx::query(
            r#"SELECT coalesce(a.directory_id,$2) AS directory_id,count(*) AS total
            FROM interface_documents d
            LEFT JOIN catalog_version_assignments a ON a.interface_id=d.id
            AND a.version_id=$3
            WHERE d.project_id=$1
            GROUP BY coalesce(a.directory_id,$2)"#,
        )
        .bind(uuid(project))
        .bind(s.system)
        .bind(s.current)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        let counts: std::collections::HashMap<Uuid, i64> = rows
            .iter()
            .map(|r| (r.get("directory_id"), r.get("total")))
            .collect();
        let mut nodes = vec![OfficialDirectory {
            id: s.system.to_string().parse().unwrap(),
            parent: None,
            name: "待分类".into(),
            description: "已正式入库、尚未分配业务目录的接口".into(),
            system: true,
            locked: true,
            direct_interfaces: *counts.get(&s.system).unwrap_or(&0),
        }];
        if let Some(c) = &candidate {
            for n in &c.nodes {
                nodes.push(OfficialDirectory {
                    id: n.id,
                    parent: n.parent,
                    name: n.name.clone(),
                    description: n.description.clone(),
                    system: false,
                    locked: false,
                    direct_interfaces: *counts.get(&uuid(n.id)).unwrap_or(&0),
                });
            }
        }
        let result = OfficialCatalog {
            project_id: project,
            generation: s.generation,
            version_id: s.current.map(|v| v.to_string().parse().unwrap()),
            source_task_id,
            source_run_id,
            unclassified_id: s.system.to_string().parse().unwrap(),
            total_interfaces: counts.values().sum(),
            nodes,
            merge_groups: candidate.map(|c| c.merge_groups).unwrap_or_default(),
        };
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
    async fn interfaces(
        &self,
        user: UserId,
        project: ProjectId,
        directory: Option<DirectoryId>,
        page: u32,
        limit: u32,
        expected_generation: Option<i64>,
    ) -> Result<OfficialInterfacePage> {
        pagination(page, limit)?;
        let mut tx = self.read_tx(user, project).await?;
        let s = state(&mut tx, project).await?;
        if expected_generation.is_some_and(|g| g != s.generation) {
            return Err(Error::Conflict);
        }
        if let Some(id) = directory
            && uuid(id) != s.system
        {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM catalog_version_nodes WHERE version_id=$1 AND id=$2)",
            )
            .bind(s.current)
            .bind(uuid(id))
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
            if !exists {
                return Err(Error::NotFound);
            }
        }
        let total: i64 = sqlx::query_scalar(
            r#"SELECT count(*)
            FROM interface_documents d
            LEFT JOIN catalog_version_assignments a ON a.interface_id=d.id
            AND a.version_id=$2
            WHERE d.project_id=$1
            AND ($4::uuid IS NULL
            OR coalesce(a.directory_id,$3)=$4)"#,
        )
        .bind(uuid(project))
        .bind(s.current)
        .bind(s.system)
        .bind(directory.map(uuid))
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        let rows=sqlx::query(r#"SELECT jsonb_build_object('interface_id',d.id,'method',d.method,'path',d.path,'directory_id',coalesce(a.directory_id,$3),'environments',(SELECT coalesce(jsonb_agg(jsonb_build_object('environment_id',e.id,'environment_name',e.name,'revision_id',c.current_revision_id)
            ORDER BY e.id),'[]'::jsonb)
            FROM interface_environment_current c
            JOIN environments e ON e.id=c.environment_id
            WHERE c.interface_id=d.id)) AS value
            FROM interface_documents d
            LEFT JOIN catalog_version_assignments a ON a.interface_id=d.id
            AND a.version_id=$2
            WHERE d.project_id=$1
            AND ($4::uuid IS NULL
            OR coalesce(a.directory_id,$3)=$4)
            ORDER BY d.method,d.path,d.id
            LIMIT $5 OFFSET $6"#)
   .bind(uuid(project)).bind(s.current).bind(s.system).bind(directory.map(uuid)).bind(i64::from(limit)).bind(i64::from(page-1)*i64::from(limit)).fetch_all(&mut *tx).await.map_err(db_error)?;
        let items = rows
            .into_iter()
            .map(|r| decode(r.get("value")))
            .collect::<Result<_>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(OfficialInterfacePage {
            items,
            page,
            limit,
            total,
            generation: s.generation,
        })
    }
    async fn versions(
        &self,
        user: UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<CatalogVersionPage> {
        pagination(page, limit)?;
        let mut tx = self.read_tx(user, project).await?;
        let s = state(&mut tx, project).await?;
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM catalog_versions WHERE project_id=$1")
                .bind(uuid(project))
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        let rows=sqlx::query(r#"SELECT jsonb_build_object('version_id',id,'source_task_id',source_task_id,'source_run_id',maintenance_run_id,'created_at',created_at,'current',coalesce(id=$2,false)) AS value
            FROM catalog_versions
            WHERE project_id=$1
            ORDER BY created_at DESC,id DESC
            LIMIT $3 OFFSET $4"#)
   .bind(uuid(project)).bind(s.current).bind(i64::from(limit)).bind(i64::from(page-1)*i64::from(limit)).fetch_all(&mut *tx).await.map_err(db_error)?;
        let items = rows
            .into_iter()
            .map(|r| decode(r.get("value")))
            .collect::<Result<_>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(CatalogVersionPage {
            items,
            page,
            limit,
            total,
            generation: s.generation,
        })
    }
}
#[async_trait]
impl CatalogActivationStore for PostgresOfficialCatalog {
    async fn publish(
        &self,
        user: UserId,
        project: ProjectId,
        request: &PublishCatalog,
        validated: &PreviewTask,
    ) -> Result<CatalogActivation> {
        decode(
            crate::publication::execute(
                &self.database,
                user,
                project,
                crate::publication::Change::DirectoryPublish(request, validated),
            )
            .await?,
        )
    }
    async fn restore(
        &self,
        user: UserId,
        project: ProjectId,
        request: &RestoreCatalog,
    ) -> Result<CatalogActivation> {
        decode(
            crate::publication::execute(
                &self.database,
                user,
                project,
                crate::publication::Change::DirectoryRestore(request),
            )
            .await?,
        )
    }
}

use crate::Postgres;
use async_trait::async_trait;
use nexofolio_application::CatalogActivationStore;
use nexofolio_contracts::*;
use nexofolio_knowledge::OfficialCatalogReader;
use serde_json::{Value, json};
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
        Err(Error::InvalidInput {
            message: "invalid pagination".into(),
        })
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
async fn state(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    project: ProjectId,
    write: bool,
) -> Result<State> {
    let sql = if write {
        "SELECT unclassified_id,current_version_id,generation FROM project_catalogs WHERE project_id=$1 FOR UPDATE"
    } else {
        "SELECT unclassified_id,current_version_id,generation FROM project_catalogs WHERE project_id=$1"
    };
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
        let s = state(&mut tx, project, false).await?;
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
        let s = state(&mut tx, project, false).await?;
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
        let s = state(&mut tx, project, false).await?;
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
async fn prior(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    user: UserId,
    project: ProjectId,
    request: Uuid,
    command: &Value,
) -> Result<Option<CatalogActivation>> {
    let row=sqlx::query("SELECT actor_id,command,result FROM catalog_activation_receipts WHERE project_id=$1 AND request_id=$2").bind(uuid(project)).bind(request).fetch_optional(&mut **tx).await.map_err(db_error)?;
    if let Some(r) = row {
        if r.get::<Uuid, _>("actor_id") != uuid(user) || r.get::<Value, _>("command") != *command {
            return Err(Error::Conflict);
        }
        let mut result: CatalogActivation = decode(r.get("result"))?;
        result.replayed = true;
        return Ok(Some(result));
    }
    Ok(None)
}
async fn activate(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    s: &State,
    user: UserId,
    project: ProjectId,
    request: Uuid,
    command: Value,
    target: Option<Uuid>,
) -> Result<CatalogActivation> {
    let generation = if s.current == target {
        s.generation
    } else {
        s.generation.checked_add(1).ok_or(Error::Conflict)?
    };
    sqlx::query(
        "UPDATE project_catalogs SET current_version_id=$2,generation=$3 WHERE project_id=$1",
    )
    .bind(uuid(project))
    .bind(target)
    .bind(generation)
    .execute(&mut **tx)
    .await
    .map_err(db_error)?;
    if s.current != target {
        crate::knowledge_publication::preserve_semantics_for_catalog(tx, user, project, target)
            .await?;
    }
    let result = CatalogActivation {
        request_id: request,
        generation,
        version_id: target.map(|v| v.to_string().parse().unwrap()),
        replayed: false,
    };
    sqlx::query("INSERT INTO catalog_activation_receipts(project_id,request_id,actor_id,command,result) VALUES($1,$2,$3,$4,$5)").bind(uuid(project)).bind(request).bind(uuid(user)).bind(command).bind(serde_json::to_value(&result).expect("serializes")).execute(&mut **tx).await.map_err(db_error)?;
    Ok(result)
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
        let command = json!({"action":"publish","request":request});
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        authorize(&mut tx, user, project).await?;
        let s = state(&mut tx, project, true).await?;
        if let Some(result) = prior(&mut tx, user, project, request.request_id, &command).await? {
            tx.commit().await.map_err(db_error)?;
            return Ok(result);
        }
        if s.generation != request.expected_generation {
            return Err(Error::Conflict);
        }
        let candidate = validated.candidate.as_ref().ok_or(Error::Conflict)?;
        let candidate_value = serde_json::to_value(candidate).expect("serializes");
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM catalog_preview_tasks WHERE id=$1 AND project_id=$2 AND status='ready' AND candidate_id=$3 AND snapshot_sha256=$4 AND candidate=$5 AND snapshot=$6)")
   .bind(uuid(request.task_id)).bind(uuid(project)).bind(uuid(validated.candidate_id)).bind(&validated.snapshot_sha256).bind(&candidate_value).bind(serde_json::to_value(&validated.snapshot).expect("serializes")).fetch_one(&mut *tx).await.map_err(db_error)?;
        if !valid {
            return Err(Error::Conflict);
        }
        if candidate.nodes.iter().any(|n| {
            n.name == "待分类"
                || uuid(n.id) == s.system
                || n.parent.is_some_and(|p| uuid(p) == s.system)
        }) {
            return Err(Error::InvalidInput {
                message: "system directory is immutable".into(),
            });
        }
        let ids: Vec<_> = candidate
            .assignments
            .iter()
            .map(|a| uuid(a.interface_id))
            .collect();
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM interface_documents WHERE project_id=$1 AND id=ANY($2)",
        )
        .bind(uuid(project))
        .bind(&ids)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        if count as usize != ids.len() {
            return Err(Error::Conflict);
        }
        let version = uuid(validated.candidate_id);
        let inserted=sqlx::query("INSERT INTO catalog_versions(id,project_id,source_task_id,candidate,created_by) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO NOTHING")
   .bind(version).bind(uuid(project)).bind(uuid(request.task_id)).bind(candidate_value).bind(uuid(user)).execute(&mut *tx).await.map_err(db_error)?.rows_affected();
        if inserted == 1 {
            for node in &candidate.nodes {
                sqlx::query("INSERT INTO catalog_version_nodes(version_id,id,parent_id,name,description) VALUES($1,$2,$3,$4,$5)").bind(version).bind(uuid(node.id)).bind(node.parent.map(uuid)).bind(&node.name).bind(&node.description).execute(&mut *tx).await.map_err(db_error)?;
            }
            for a in &candidate.assignments {
                sqlx::query("INSERT INTO catalog_version_assignments(version_id,interface_id,directory_id) VALUES($1,$2,$3)").bind(version).bind(uuid(a.interface_id)).bind(a.directory_id.map(uuid)).execute(&mut *tx).await.map_err(db_error)?;
            }
        }
        let result = activate(
            &mut tx,
            &s,
            user,
            project,
            request.request_id,
            command,
            Some(version),
        )
        .await?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
    async fn restore(
        &self,
        user: UserId,
        project: ProjectId,
        request: &RestoreCatalog,
    ) -> Result<CatalogActivation> {
        let command = json!({"action":"restore","request":request});
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        authorize(&mut tx, user, project).await?;
        let s = state(&mut tx, project, true).await?;
        if let Some(result) = prior(&mut tx, user, project, request.request_id, &command).await? {
            tx.commit().await.map_err(db_error)?;
            return Ok(result);
        }
        if s.generation != request.expected_generation {
            return Err(Error::Conflict);
        }
        if let Some(version) = request.version_id {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM catalog_versions WHERE id=$1 AND project_id=$2)",
            )
            .bind(uuid(version))
            .bind(uuid(project))
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
            if !exists {
                return Err(Error::NotFound);
            }
        }
        let result = activate(
            &mut tx,
            &s,
            user,
            project,
            request.request_id,
            command,
            request.version_id.map(uuid),
        )
        .await?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
}

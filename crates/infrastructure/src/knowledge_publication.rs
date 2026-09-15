use crate::maintenance_store::*;
use async_trait::async_trait;
use nexofolio_application::KnowledgeActivationStore;
use nexofolio_contracts::*;
use nexofolio_rebuild::materialize_maintenance;
use serde_json::{Value, json};
use sqlx::{Row, Transaction};
use uuid::Uuid;

pub(crate) async fn preserve_semantics_for_catalog(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    user: UserId,
    project: ProjectId,
    target: Option<Uuid>,
) -> Result<()> {
    let annotations:Value=sqlx::query_scalar("SELECT coalesce((SELECT k.annotations FROM knowledge_releases k WHERE k.id=p.current_knowledge_version_id),'[]'::jsonb) FROM project_catalogs p WHERE p.project_id=$1").bind(id(project)).fetch_one(&mut **tx).await.map_err(db)?;
    let release = Uuid::new_v4();
    sqlx::query("INSERT INTO knowledge_releases(id,project_id,catalog_version_id,annotations,actor_id,origin) VALUES($1,$2,$3,$4,$5,'directory_only')").bind(release).bind(id(project)).bind(target).bind(annotations).bind(id(user)).execute(&mut **tx).await.map_err(db)?;
    sqlx::query("UPDATE project_catalogs SET current_knowledge_version_id=$2 WHERE project_id=$1")
        .bind(id(project))
        .bind(release)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    Ok(())
}
struct Current {
    generation: i64,
    catalog: Option<Uuid>,
    knowledge: Option<Uuid>,
}
async fn current(tx: &mut Transaction<'_, sqlx::Postgres>, project: ProjectId) -> Result<Current> {
    let r=sqlx::query("SELECT generation,current_version_id,current_knowledge_version_id FROM project_catalogs WHERE project_id=$1 FOR UPDATE").bind(id(project)).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(Error::NotFound)?;
    Ok(Current {
        generation: r.get("generation"),
        catalog: r.get("current_version_id"),
        knowledge: r.get("current_knowledge_version_id"),
    })
}
async fn prior(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    user: UserId,
    project: ProjectId,
    request: Uuid,
    command: &Value,
) -> Result<Option<KnowledgeActivation>> {
    let r=sqlx::query("SELECT command,result FROM knowledge_activation_receipts WHERE project_id=$1 AND actor_id=$2 AND request_id=$3").bind(id(project)).bind(id(user)).bind(request).fetch_optional(&mut **tx).await.map_err(db)?;
    if let Some(r) = r {
        if r.get::<Value, _>("command") != *command {
            return Err(Error::Conflict);
        }
        let mut out: KnowledgeActivation = decode(r.get("result"))?;
        out.replayed = true;
        Ok(Some(out))
    } else {
        Ok(None)
    }
}
async fn activate(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    user: UserId,
    project: ProjectId,
    s: &Current,
    request: Uuid,
    command: &Value,
    target: (Option<Uuid>, Option<Uuid>),
) -> Result<KnowledgeActivation> {
    let (catalog, knowledge) = target;
    let changed = s.catalog != catalog || s.knowledge != knowledge;
    let generation = s
        .generation
        .checked_add(i64::from(changed))
        .ok_or(Error::Conflict)?;
    sqlx::query("UPDATE project_catalogs SET current_version_id=$2,current_knowledge_version_id=$3,generation=$4 WHERE project_id=$1").bind(id(project)).bind(catalog).bind(knowledge).bind(generation).execute(&mut **tx).await.map_err(db)?;
    let out = KnowledgeActivation {
        request_id: request,
        generation,
        version_id: knowledge,
        catalog_version_id: catalog.map(|v| v.to_string().parse().unwrap()),
        changed,
        replayed: false,
    };
    sqlx::query("INSERT INTO knowledge_activation_receipts(project_id,actor_id,request_id,command,result) VALUES($1,$2,$3,$4,$5)").bind(id(project)).bind(id(user)).bind(request).bind(command).bind(encode(&out)).execute(&mut **tx).await.map_err(db)?;
    Ok(out)
}
#[async_trait]
impl KnowledgeActivationStore for PostgresMaintenance {
    async fn publish_knowledge(
        &self,
        user: UserId,
        project: ProjectId,
        run: Uuid,
        request: &PublishKnowledge,
    ) -> Result<KnowledgeActivation> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        auth(&mut tx, user, project).await?;
        let s = current(&mut tx, project).await?;
        let command = json!({"action":"publish","run":run,"request":request});
        if let Some(out) = prior(&mut tx, user, project, request.request_id, &command).await? {
            tx.commit().await.map_err(db)?;
            return Ok(out);
        }
        if request.expected_generation != s.generation {
            return Err(Error::Conflict);
        }
        let row=sqlx::query("SELECT status,snapshot,candidate FROM maintenance_runs WHERE id=$1 AND project_id=$2 FOR SHARE").bind(run).bind(id(project)).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        if row.get::<String, _>("status") != "ready" {
            return Err(invalid("maintenance candidate is not ready"));
        }
        let snapshot: KnowledgeSnapshot = decode(row.get("snapshot"))?;
        let candidate: MaintenanceCandidate = decode(row.get("candidate"))?;
        if snapshot.base_generation != s.generation {
            return Err(Error::Conflict);
        }
        let checked =
            materialize_maintenance(&snapshot, &candidate.plan, run, candidate.coverage.clone())?;
        if !checked.issues.is_empty()
            || !checked.review.structurally_valid
            || encode(&checked) != encode(&candidate)
        {
            return Err(invalid("candidate failed publication checks"));
        }
        // Lock current definitions while validating revision preconditions. New interfaces are not removed.
        let rows=sqlx::query("SELECT interface_id,environment_id,current_revision_id FROM interface_environment_current WHERE interface_id=ANY($1) FOR SHARE").bind(snapshot.interfaces.iter().map(|i|id(i.interface_id)).collect::<Vec<_>>()).fetch_all(&mut *tx).await.map_err(db)?;
        for i in &snapshot.interfaces {
            for e in &i.environments {
                if !rows.iter().any(|r| {
                    r.get::<Uuid, _>("interface_id") == id(i.interface_id)
                        && r.get::<Uuid, _>("environment_id") == id(e.environment_id)
                        && r.get::<Uuid, _>("current_revision_id") == id(e.revision_id)
                }) {
                    return Err(Error::Conflict);
                }
            }
        }
        let catalog_changed = encode(&candidate.catalog) != encode(&snapshot.catalog);
        let semantic_changed = encode(&candidate.annotations) != encode(&snapshot.annotations);
        let (catalog, knowledge) = if !catalog_changed && !semantic_changed {
            (s.catalog, s.knowledge)
        } else {
            let catalog = if catalog_changed {
                let v = Uuid::new_v4();
                sqlx::query("INSERT INTO catalog_versions(id,project_id,maintenance_run_id,candidate,created_by) VALUES($1,$2,$3,$4,$5)").bind(v).bind(id(project)).bind(run).bind(encode(&candidate.catalog)).bind(id(user)).execute(&mut *tx).await.map_err(db)?;
                for node in &candidate.catalog.nodes {
                    sqlx::query("INSERT INTO catalog_version_nodes(version_id,id,parent_id,name,description) VALUES($1,$2,$3,$4,$5)").bind(v).bind(id(node.id)).bind(node.parent.map(id)).bind(&node.name).bind(&node.description).execute(&mut *tx).await.map_err(db)?;
                }
                for a in &candidate.catalog.assignments {
                    sqlx::query("INSERT INTO catalog_version_assignments(version_id,interface_id,directory_id) VALUES($1,$2,$3)").bind(v).bind(id(a.interface_id)).bind(a.directory_id.map(id)).execute(&mut *tx).await.map_err(db)?;
                }
                Some(v)
            } else {
                s.catalog
            };
            let release = Uuid::new_v4();
            sqlx::query("INSERT INTO knowledge_releases(id,project_id,catalog_version_id,source_run_id,annotations,actor_id,origin) VALUES($1,$2,$3,$4,$5,$6,'maintenance')").bind(release).bind(id(project)).bind(catalog).bind(run).bind(encode(&candidate.annotations)).bind(id(user)).execute(&mut *tx).await.map_err(db)?;
            (catalog, Some(release))
        };
        let out = activate(
            &mut tx,
            user,
            project,
            &s,
            request.request_id,
            &command,
            (catalog, knowledge),
        )
        .await?;
        tx.commit().await.map_err(db)?;
        Ok(out)
    }
    async fn restore_knowledge(
        &self,
        user: UserId,
        project: ProjectId,
        request: &RestoreKnowledge,
    ) -> Result<KnowledgeActivation> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        auth(&mut tx, user, project).await?;
        let s = current(&mut tx, project).await?;
        let command = json!({"action":"restore","request":request});
        if let Some(out) = prior(&mut tx, user, project, request.request_id, &command).await? {
            tx.commit().await.map_err(db)?;
            return Ok(out);
        }
        if request.expected_generation != s.generation {
            return Err(Error::Conflict);
        }
        let catalog = if let Some(v) = request.version_id {
            sqlx::query_scalar::<_, Option<Uuid>>(
                "SELECT catalog_version_id FROM knowledge_releases WHERE id=$1 AND project_id=$2",
            )
            .bind(v)
            .bind(id(project))
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .ok_or(Error::NotFound)?
        } else {
            None
        };
        let out = activate(
            &mut tx,
            user,
            project,
            &s,
            request.request_id,
            &command,
            (catalog, request.version_id),
        )
        .await?;
        tx.commit().await.map_err(db)?;
        Ok(out)
    }
    async fn knowledge_versions(
        &self,
        user: UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<KnowledgeVersionPage> {
        page_check(page, limit)?;
        let mut tx = self.read_tx(user, project).await?;
        let generation =
            sqlx::query_scalar("SELECT generation FROM project_catalogs WHERE project_id=$1")
                .bind(id(project))
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        let total =
            sqlx::query_scalar("SELECT count(*) FROM knowledge_releases WHERE project_id=$1")
                .bind(id(project))
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        let items=sqlx::query_scalar(r#"SELECT to_jsonb(k)-'annotations'-'actor_id' || jsonb_build_object('current',coalesce(k.id=p.current_knowledge_version_id,false))
            FROM knowledge_releases k
            JOIN project_catalogs p ON p.project_id=k.project_id
            WHERE k.project_id=$1
            ORDER BY k.created_at DESC,k.id
            LIMIT $2 OFFSET $3"#).bind(id(project)).bind(i64::from(limit)).bind(i64::from(page-1)*i64::from(limit)).fetch_all(&mut *tx).await.map_err(db)?.into_iter().map(decode).collect::<Result<_>>()?;
        tx.commit().await.map_err(db)?;
        Ok(KnowledgeVersionPage {
            items,
            total,
            page,
            limit,
            generation,
        })
    }
    async fn interface_knowledge(
        &self,
        user: UserId,
        project: ProjectId,
        interface: InterfaceId,
        environment: EnvironmentId,
    ) -> Result<InterfaceKnowledge> {
        let mut tx = self.read_tx(user, project).await?;
        let r=sqlx::query(r#"SELECT c.current_revision_id,p.generation,p.current_knowledge_version_id,coalesce(k.annotations,'[]'::jsonb) AS annotations
            FROM interface_documents d
            JOIN interface_environment_current c ON c.interface_id=d.id
            JOIN project_catalogs p ON p.project_id=d.project_id
            LEFT JOIN knowledge_releases k ON k.id=p.current_knowledge_version_id
            WHERE d.id=$1
            AND d.project_id=$2
            AND c.environment_id=$3"#).bind(id(interface)).bind(id(project)).bind(id(environment)).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let annotations: Vec<SemanticAnnotation> = decode(r.get("annotations"))?;
        let mut annotations: Vec<_> = annotations
            .into_iter()
            .filter(|a| match &a.annotation.target {
                SemanticTarget::Interface { interface_id } => *interface_id == interface,
                SemanticTarget::Field { field } => {
                    field.interface_id == interface && field.environment_id == environment
                }
            })
            .collect();
        let bases = annotations
            .iter()
            .flat_map(|a| a.basis.iter().map(|b| id(b.interface_id)))
            .collect::<Vec<_>>();
        let revisions=sqlx::query("SELECT interface_id,environment_id,current_revision_id FROM interface_environment_current WHERE interface_id=ANY($1)").bind(bases).fetch_all(&mut *tx).await.map_err(db)?;
        for a in &mut annotations {
            a.stale = a.basis.iter().any(|b| {
                !revisions.iter().any(|v| {
                    v.get::<Uuid, _>("interface_id") == id(b.interface_id)
                        && v.get::<Uuid, _>("environment_id") == id(b.environment_id)
                        && v.get::<Uuid, _>("current_revision_id") == id(b.revision_id)
                })
            });
        }
        let out = InterfaceKnowledge {
            interface_id: interface,
            environment_id: environment,
            revision_id: r
                .get::<Uuid, _>("current_revision_id")
                .to_string()
                .parse()
                .unwrap(),
            generation: r.get("generation"),
            knowledge_version_id: r.get("current_knowledge_version_id"),
            annotations,
        };
        tx.commit().await.map_err(db)?;
        Ok(out)
    }
}

//! One project version activation transaction; compatibility differences are explicit intents.
use crate::project_access::authorize_project;
use crate::{
    Postgres,
    maintenance_store::{db, decode, encode, id, invalid},
};
use nexofolio_contracts::*;
use nexofolio_rebuild::materialize_maintenance;
use serde_json::{Value, json};
use sqlx::{Row, Transaction};
use uuid::Uuid;

pub(crate) enum Change<'a> {
    DirectoryPublish(&'a PublishCatalog, &'a PreviewTask),
    DirectoryRestore(&'a RestoreCatalog),
    KnowledgePublish(Uuid, &'a PublishKnowledge),
    KnowledgeRestore(&'a RestoreKnowledge),
}
struct Current {
    system: Uuid,
    catalog: Option<Uuid>,
    knowledge: Option<Uuid>,
    generation: i64,
}

pub(crate) async fn execute(
    database: &Postgres,
    user: UserId,
    project: ProjectId,
    change: Change<'_>,
) -> Result<Value> {
    let (request, expected, command, directory_only) = match &change {
        Change::DirectoryPublish(q, _) => (
            q.request_id,
            q.expected_generation,
            json!({"action":"publish","request":q}),
            true,
        ),
        Change::DirectoryRestore(q) => (
            q.request_id,
            q.expected_generation,
            json!({"action":"restore","request":q}),
            true,
        ),
        Change::KnowledgePublish(run, q) => (
            q.request_id,
            q.expected_generation,
            json!({"action":"publish","run":run,"request":q}),
            false,
        ),
        Change::KnowledgeRestore(q) => (
            q.request_id,
            q.expected_generation,
            json!({"action":"restore","request":q}),
            false,
        ),
    };
    let mut tx = database.pool.begin().await.map_err(db)?;
    authorize_project(&mut tx, user, project).await?;
    let row=sqlx::query("SELECT unclassified_id,current_version_id,current_knowledge_version_id,generation FROM project_catalogs WHERE project_id=$1 FOR UPDATE")
        .bind(id(project)).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
    let state = Current {
        system: row.get("unclassified_id"),
        catalog: row.get("current_version_id"),
        knowledge: row.get("current_knowledge_version_id"),
        generation: row.get("generation"),
    };
    // Preserve the original receipt namespaces: legacy IDs are project-wide; knowledge IDs are actor-scoped.
    let lookup = if directory_only {
        "SELECT actor_id,command,result FROM catalog_activation_receipts WHERE project_id=$1 AND request_id=$2"
    } else {
        "SELECT actor_id,command,result FROM knowledge_activation_receipts WHERE project_id=$1 AND request_id=$2 AND actor_id=$3"
    };
    let mut lookup = sqlx::query(lookup).bind(id(project)).bind(request);
    if !directory_only {
        lookup = lookup.bind(id(user));
    }
    if let Some(row) = lookup.fetch_optional(&mut *tx).await.map_err(db)? {
        if row.get::<Uuid, _>("actor_id") != id(user) || row.get::<Value, _>("command") != command {
            return Err(Error::Conflict);
        }
        let receipt = checked_receipt(row.get("result"), directory_only, true)?;
        tx.commit().await.map_err(db)?;
        return Ok(receipt);
    }
    if expected != state.generation {
        return Err(Error::Conflict);
    }
    let (catalog, mut knowledge) = match change {
        Change::DirectoryPublish(q, task) => (
            Some(prepare_directory(&mut tx, &state, user, project, q, task).await?),
            state.knowledge,
        ),
        Change::DirectoryRestore(q) => {
            let target = q.version_id.map(id);
            if let Some(version) = target {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM catalog_versions WHERE id=$1 AND project_id=$2)",
                )
                .bind(version)
                .bind(id(project))
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
                if !exists {
                    return Err(Error::NotFound);
                }
            }
            (target, state.knowledge)
        }
        Change::KnowledgePublish(run, _) => {
            prepare_knowledge(&mut tx, &state, user, project, run).await?
        }
        Change::KnowledgeRestore(q) => {
            let target = if let Some(version) = q.version_id {
                sqlx::query_scalar::<_,Option<Uuid>>("SELECT catalog_version_id FROM knowledge_releases WHERE id=$1 AND project_id=$2")
                    .bind(version).bind(id(project)).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?
            } else {
                None
            };
            (target, q.version_id)
        }
    };
    if directory_only && catalog != state.catalog {
        let annotations: Value = sqlx::query_scalar(
            "SELECT coalesce((SELECT annotations FROM knowledge_releases WHERE id=$1),'[]'::jsonb)",
        )
        .bind(state.knowledge)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        let release = Uuid::new_v4();
        sqlx::query("INSERT INTO knowledge_releases(id,project_id,catalog_version_id,annotations,actor_id,origin) VALUES($1,$2,$3,$4,$5,'directory_only')")
            .bind(release).bind(id(project)).bind(catalog).bind(annotations).bind(id(user)).execute(&mut *tx).await.map_err(db)?;
        knowledge = Some(release);
    }
    let changed = (catalog, knowledge) != (state.catalog, state.knowledge);
    let generation = state
        .generation
        .checked_add(i64::from(changed))
        .ok_or(Error::Conflict)?;
    sqlx::query("UPDATE project_catalogs SET current_version_id=$2,current_knowledge_version_id=$3,generation=$4 WHERE project_id=$1")
        .bind(id(project)).bind(catalog).bind(knowledge).bind(generation).execute(&mut *tx).await.map_err(db)?;
    let receipt = if directory_only {
        json!({"request_id":request,"generation":generation,"version_id":catalog,"replayed":false})
    } else {
        json!({"request_id":request,"generation":generation,"version_id":knowledge,"catalog_version_id":catalog,"changed":changed,"replayed":false})
    };
    let receipt = checked_receipt(receipt, directory_only, false)?;
    let insert = if directory_only {
        "INSERT INTO catalog_activation_receipts(project_id,actor_id,request_id,command,result) VALUES($1,$2,$3,$4,$5)"
    } else {
        "INSERT INTO knowledge_activation_receipts(project_id,actor_id,request_id,command,result) VALUES($1,$2,$3,$4,$5)"
    };
    sqlx::query(insert)
        .bind(id(project))
        .bind(id(user))
        .bind(request)
        .bind(command)
        .bind(&receipt)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(receipt)
}

async fn save_catalog(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    user: UserId,
    project: ProjectId,
    candidate: &DirectoryCandidate,
    version: Uuid,
    source: (Option<Uuid>, Option<Uuid>),
) -> Result<Uuid> {
    let inserted=sqlx::query("INSERT INTO catalog_versions(id,project_id,source_task_id,maintenance_run_id,candidate,created_by) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING")
        .bind(version).bind(id(project)).bind(source.0).bind(source.1).bind(encode(candidate)).bind(id(user)).execute(&mut **tx).await.map_err(db)?.rows_affected();
    if inserted == 0 && source.1.is_some() {
        return Err(Error::Conflict);
    }
    if inserted == 1 {
        for node in &candidate.nodes {
            sqlx::query("INSERT INTO catalog_version_nodes(version_id,id,parent_id,name,description) VALUES($1,$2,$3,$4,$5)")
                .bind(version).bind(id(node.id)).bind(node.parent.map(id)).bind(&node.name).bind(&node.description).execute(&mut **tx).await.map_err(db)?;
        }
        for a in &candidate.assignments {
            sqlx::query("INSERT INTO catalog_version_assignments(version_id,interface_id,directory_id) VALUES($1,$2,$3)")
                .bind(version).bind(id(a.interface_id)).bind(a.directory_id.map(id)).execute(&mut **tx).await.map_err(db)?;
        }
    }
    Ok(version)
}
async fn prepare_directory(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    state: &Current,
    user: UserId,
    project: ProjectId,
    request: &PublishCatalog,
    validated: &PreviewTask,
) -> Result<Uuid> {
    let candidate = validated.candidate.as_ref().ok_or(Error::Conflict)?;
    let candidate_value = serde_json::to_value(candidate).expect("serializes");
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM catalog_preview_tasks WHERE id=$1 AND project_id=$2 AND status='ready' AND candidate_id=$3 AND snapshot_sha256=$4 AND candidate=$5 AND snapshot=$6)")
   .bind(id(request.task_id)).bind(id(project)).bind(id(validated.candidate_id)).bind(&validated.snapshot_sha256).bind(&candidate_value).bind(serde_json::to_value(&validated.snapshot).expect("serializes")).fetch_one(&mut **tx).await.map_err(db)?;
    if !valid {
        return Err(Error::Conflict);
    }
    if candidate.nodes.iter().any(|n| {
        n.name == "待分类"
            || id(n.id) == state.system
            || n.parent.is_some_and(|p| id(p) == state.system)
    }) {
        return Err(Error::invalid("system directory is immutable"));
    }
    let ids: Vec<_> = candidate
        .assignments
        .iter()
        .map(|a| id(a.interface_id))
        .collect();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM interface_documents WHERE project_id=$1 AND id=ANY($2)",
    )
    .bind(id(project))
    .bind(&ids)
    .fetch_one(&mut **tx)
    .await
    .map_err(db)?;
    if count as usize != ids.len() {
        return Err(Error::Conflict);
    }

    save_catalog(
        tx,
        user,
        project,
        candidate,
        id(validated.candidate_id),
        (Some(id(request.task_id)), None),
    )
    .await
}
async fn prepare_knowledge(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    state: &Current,
    user: UserId,
    project: ProjectId,
    run: Uuid,
) -> Result<(Option<Uuid>, Option<Uuid>)> {
    let row=sqlx::query("SELECT status,snapshot,candidate FROM maintenance_runs WHERE id=$1 AND project_id=$2 FOR SHARE").bind(run).bind(id(project)).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(Error::NotFound)?;
    if row.get::<String, _>("status") != "ready" {
        return Err(invalid("maintenance candidate is not ready"));
    }
    let snapshot: KnowledgeSnapshot = decode(row.get("snapshot"))?;
    let candidate: MaintenanceCandidate = decode(row.get("candidate"))?;
    if snapshot.base_generation != state.generation {
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
    let rows=sqlx::query("SELECT interface_id,environment_id,current_revision_id FROM interface_environment_current WHERE interface_id=ANY($1) FOR SHARE").bind(snapshot.interfaces.iter().map(|i|id(i.interface_id)).collect::<Vec<_>>()).fetch_all(&mut **tx).await.map_err(db)?;
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
        (state.catalog, state.knowledge)
    } else {
        let catalog = if catalog_changed {
            let v = save_catalog(
                tx,
                user,
                project,
                &candidate.catalog,
                Uuid::new_v4(),
                (None, Some(run)),
            )
            .await?;
            Some(v)
        } else {
            state.catalog
        };
        let release = Uuid::new_v4();
        sqlx::query("INSERT INTO knowledge_releases(id,project_id,catalog_version_id,source_run_id,annotations,actor_id,origin) VALUES($1,$2,$3,$4,$5,$6,'maintenance')").bind(release).bind(id(project)).bind(catalog).bind(run).bind(encode(&candidate.annotations)).bind(id(user)).execute(&mut **tx).await.map_err(db)?;
        (catalog, Some(release))
    };
    Ok((catalog, knowledge))
}

// Validate the external receipt shape before committing, including historical replays.
fn checked_receipt(value: Value, directory_only: bool, replayed: bool) -> Result<Value> {
    if directory_only {
        let mut receipt: CatalogActivation = decode(value)?;
        receipt.replayed = replayed;
        Ok(encode(&receipt))
    } else {
        let mut receipt: KnowledgeActivation = decode(value)?;
        receipt.replayed = replayed;
        Ok(encode(&receipt))
    }
}

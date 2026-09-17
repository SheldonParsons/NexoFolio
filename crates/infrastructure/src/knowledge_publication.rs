use crate::maintenance_store::*;
use async_trait::async_trait;
use nexofolio_application::KnowledgeActivationStore;
use nexofolio_contracts::*;
use sqlx::Row;
use uuid::Uuid;
#[async_trait]
impl KnowledgeActivationStore for PostgresMaintenance {
    async fn publish_knowledge(
        &self,
        user: UserId,
        project: ProjectId,
        run: Uuid,
        request: &PublishKnowledge,
    ) -> Result<KnowledgeActivation> {
        decode(
            crate::publication::execute(
                &self.database,
                user,
                project,
                crate::publication::Change::KnowledgePublish(run, request),
            )
            .await?,
        )
    }
    async fn restore_knowledge(
        &self,
        user: UserId,
        project: ProjectId,
        request: &RestoreKnowledge,
    ) -> Result<KnowledgeActivation> {
        decode(
            crate::publication::execute(
                &self.database,
                user,
                project,
                crate::publication::Change::KnowledgeRestore(request),
            )
            .await?,
        )
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

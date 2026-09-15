use super::*;
impl PostgresMaintenance {
    pub(super) async fn start_snapshot(
        &self,
        user: UserId,
        project: ProjectId,
        request: &StartMaintenance,
    ) -> Result<MaintenanceRun> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        // Take the evidence lock before establishing a repeatable-read snapshot.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "maintenance-start:{project}:{}",
                request.request_id
            ))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        auth(&mut tx, user, project).await?;
        if let Some(run) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM maintenance_runs WHERE actor_id=$1 AND project_id=$2 AND request_id=$3",
        )
        .bind(id(user))
        .bind(id(project))
        .bind(request.request_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        {
            let out = Self::run_tx(&mut tx, project, run).await?;
            tx.commit().await.map_err(db)?;
            return Ok(out);
        }
        // Serialize with evidence updates and retention; published state stays locked until snapshot commit.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("evidence-project:{project}"))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let state=sqlx::query("SELECT unclassified_id,current_version_id,current_knowledge_version_id,generation FROM project_catalogs WHERE project_id=$1 FOR SHARE").bind(id(project)).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let current: Option<Uuid> = state.get("current_version_id");
        let knowledge: Option<Uuid> = state.get("current_knowledge_version_id");
        let estimated:i64=sqlx::query_scalar(r#"SELECT (SELECT coalesce(sum(octet_length(data::text)+octet_length(subject::text)+512),0)::bigint
            FROM evidence_facts
            WHERE project_id=$1)+(SELECT coalesce(sum(octet_length(r.definition::text)),0)::bigint
            FROM interface_documents d
            JOIN interface_environment_current c ON c.interface_id=d.id
            JOIN interface_observed_revisions r ON r.id=c.current_revision_id
            WHERE d.project_id=$1)"#).bind(id(project)).fetch_one(&mut *tx).await.map_err(db)?;
        if estimated > self.max_snapshot_bytes as i64 {
            return Err(invalid("SNAPSHOT_TOO_LARGE"));
        }
        // One SQL statement captures every interface/revision atomically, including all environments.
        let rows=sqlx::query(r#"SELECT jsonb_build_object('interface_id',d.id,'method',d.method,'path',d.path,'environments',jsonb_agg(jsonb_build_object('environment_id',e.id,'environment_name',e.name,'revision_id',r.id,'definition',r.definition)
            ORDER BY e.id)) AS value
            FROM interface_documents d
            JOIN interface_environment_current c ON c.interface_id=d.id
            JOIN interface_observed_revisions r ON r.id=c.current_revision_id
            JOIN environments e ON e.id=c.environment_id
            WHERE d.project_id=$1
            GROUP BY d.id,d.method,d.path
            ORDER BY d.method,d.path,d.id"#)
   .bind(id(project)).fetch_all(&mut *tx).await.map_err(db)?;
        let mut interfaces = rows
            .into_iter()
            .map(|r| decode::<CatalogInterface>(r.get("value")))
            .collect::<Result<Vec<_>>>()?;
        if interfaces.is_empty() {
            return Err(invalid("project has no observed interfaces"));
        }
        let policy: PathPolicy = decode(
            sqlx::query_scalar("SELECT path_policy FROM projects WHERE id=$1")
                .bind(id(project))
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?,
        )?;
        for i in &mut interfaces {
            i.recognized_path = Some(identify_path(&i.path, &policy));
        }
        let mut catalog: DirectoryCandidate = if let Some(v) = current {
            decode(
                sqlx::query_scalar("SELECT candidate FROM catalog_versions WHERE id=$1")
                    .bind(v)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db)?,
            )?
        } else {
            DirectoryCandidate {
                nodes: vec![],
                assignments: vec![],
                merge_groups: vec![],
            }
        };
        for i in &interfaces {
            if !catalog
                .assignments
                .iter()
                .any(|a| a.interface_id == i.interface_id)
            {
                catalog.assignments.push(PreviewAssignment {
                    interface_id: i.interface_id,
                    directory_id: None,
                    reason: "新接口保留在固定待分类".into(),
                });
            }
        }
        let mut annotations: Vec<SemanticAnnotation> = if let Some(v) = knowledge {
            decode(
                sqlx::query_scalar("SELECT annotations FROM knowledge_releases WHERE id=$1")
                    .bind(v)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db)?,
            )?
        } else {
            vec![]
        };
        mark_stale(&mut annotations, &interfaces);
        let facts=sqlx::query(r#"SELECT to_jsonb(f)-'key_hash'-'subject_hash' || jsonb_build_object('samples',coalesce((SELECT jsonb_agg(s.event_id
            ORDER BY s.event_id)
            FROM evidence_samples s
            WHERE s.fact_id=f.id),'[]'::jsonb)) AS value
            FROM evidence_facts f
            WHERE project_id=$1
            ORDER BY id"#).bind(id(project)).fetch_all(&mut *tx).await.map_err(db)?.into_iter().map(|r|decode::<EvidenceFact>(r.get("value"))).collect::<Result<Vec<_>>>()?;
        let pending:i64=sqlx::query_scalar("SELECT count(*) FROM capture_events WHERE project_id=$1 AND evidence_status<>'completed'").bind(id(project)).fetch_one(&mut *tx).await.map_err(db)?;
        let directory_metrics = nexofolio_rebuild::DirectoryReviewer::review(
            &nexofolio_rebuild::StructuralDirectoryReviewer,
            &CatalogSnapshot {
                project_id: project,
                project_name: "snapshot".into(),
                interfaces: interfaces.clone(),
            },
            &catalog,
        )
        .metrics;
        let previous_maintenance:Option<Value>=sqlx::query_scalar(r#"SELECT jsonb_build_object('run_id',m.id,'release_id',k.id,'plan',m.candidate->'plan','currently_active',coalesce(k.id=$2,false),'independent_evidence',false)
            FROM knowledge_releases k
            JOIN maintenance_runs m ON m.id=k.source_run_id
            WHERE k.project_id=$1
            ORDER BY k.created_at DESC,k.id
            LIMIT 1"#).bind(id(project)).bind(knowledge).fetch_optional(&mut *tx).await.map_err(db)?;
        let fields = snapshot_fields(&interfaces, &annotations);
        let snapshot = KnowledgeSnapshot {
            id: Uuid::new_v4(),
            project_id: project,
            base_generation: state.get("generation"),
            base_catalog_version: current.map(|v| v.to_string().parse().unwrap()),
            base_knowledge_version: knowledge,
            system_directory_id: state
                .get::<Uuid, _>("unclassified_id")
                .to_string()
                .parse()
                .unwrap(),
            catalog,
            interfaces,
            fields,
            annotations,
            facts,
            pending_observations: pending,
            directory_metrics: Some(directory_metrics),
            previous_maintenance,
        };
        if serde_json::to_vec(&snapshot).expect("serializes").len() > self.max_snapshot_bytes {
            return Err(invalid("SNAPSHOT_TOO_LARGE"));
        }
        let coverage = ReviewCoverage {
            total_interfaces: snapshot.interfaces.len(),
            total_fields: snapshot.fields.len(),
            reviewed_fields: 0,
            total_segments: 0,
            completed_segments: 0,
            complete: false,
        };
        let run = Uuid::new_v4();
        sqlx::query("INSERT INTO maintenance_runs(id,project_id,actor_id,request_id,snapshot_id,snapshot,snapshot_sha256,base_generation,coverage) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(run).bind(id(project)).bind(id(user)).bind(request.request_id).bind(snapshot.id).bind(encode(&snapshot)).bind(hash(&snapshot)).bind(snapshot.base_generation).bind(encode(&coverage)).execute(&mut *tx).await.map_err(db)?;
        for event in snapshot
            .facts
            .iter()
            .flat_map(|f| &f.samples)
            .collect::<std::collections::HashSet<_>>()
        {
            sqlx::query("INSERT INTO evidence_pins(owner_kind,owner_id,event_id) VALUES('maintenance_snapshot',$1,$2) ON CONFLICT DO NOTHING").bind(run).bind(event).execute(&mut *tx).await.map_err(db)?;
        }
        let out = Self::run_tx(&mut tx, project, run).await?;
        tx.commit().await.map_err(db)?;
        Ok(out)
    }
}

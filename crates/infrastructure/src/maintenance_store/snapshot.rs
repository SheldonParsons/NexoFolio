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
        let (mut interfaces, facts, inputs) = self.inputs(&mut tx, project).await?;
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
        let pending = inputs.gaps.get("evidence_pending").copied().unwrap_or(0)
            + inputs.gaps.get("evidence_failed").copied().unwrap_or(0);
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
        let mut snapshot = KnowledgeSnapshot {
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
            pending_observations: pending as i64,
            directory_metrics: Some(directory_metrics),
            previous_maintenance,
            inputs: Some(inputs),
        };
        snapshot.fields = nexofolio_rebuild::complete_snapshot_fields(&snapshot);
        let unavailable = snapshot
            .fields
            .iter()
            .filter(|f| f.reference.observation.is_some() && f.schema_pointers.is_empty())
            .count();
        snapshot.inputs.as_mut().unwrap().gaps.insert(
            "observation_fields_without_definition".into(),
            unavailable as u64,
        );
        let known: std::collections::HashSet<_> =
            snapshot.fields.iter().map(|f| f.id.clone()).collect();
        let unresolved: std::collections::HashSet<_> = snapshot
            .facts
            .iter()
            .flat_map(|f| nexofolio_rebuild::evidence_field_refs(&f.subject))
            .map(|r| nexofolio_rebuild::field_id(&r))
            .filter(|id| !known.contains(id))
            .collect();
        snapshot
            .inputs
            .as_mut()
            .unwrap()
            .gaps
            .insert("unresolved_fact_fields".into(), unresolved.len() as u64);
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
        let pins: Vec<_> = snapshot
            .inputs
            .as_ref()
            .unwrap()
            .sources
            .iter()
            .filter(|s| s.raw_hash.is_some())
            .map(|s| s.event_id)
            .collect();
        sqlx::query("INSERT INTO evidence_pins(owner_kind,owner_id,event_id) SELECT 'maintenance_snapshot',$1,unnest($2::uuid[]) ON CONFLICT DO NOTHING").bind(run).bind(pins).execute(&mut *tx).await.map_err(db)?;
        let out = Self::run_tx(&mut tx, project, run).await?;
        tx.commit().await.map_err(db)?;
        Ok(out)
    }
}

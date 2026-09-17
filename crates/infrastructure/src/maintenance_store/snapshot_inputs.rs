use super::*;
const SOURCE_FILTER: &str = "e.project_id=$1 AND (e.evidence_status<>'completed' OR EXISTS(SELECT 1 FROM evidence_samples s JOIN evidence_facts f ON f.id=s.fact_id WHERE s.event_id=e.id AND f.project_id=e.project_id) OR EXISTS(SELECT 1 FROM ingestion_inbox i JOIN interface_observations o ON o.ingestion_id=i.id WHERE i.project_id=e.project_id AND i.actor_id=e.actor_id AND i.producer_id=e.producer_id AND i.record_id=e.record_id))";
impl PostgresMaintenance {
    /// One statement fixes the interfaces, materials, evidence and source membership at the same MVCC cut.
    pub(super) async fn inputs(
        &self,
        tx: &mut Transaction<'_, sqlx::Postgres>,
        project: ProjectId,
    ) -> Result<(Vec<CatalogInterface>, Vec<EvidenceFact>, SnapshotInputs)> {
        let estimated:i64=sqlx::query_scalar(sqlx::AssertSqlSafe(format!(r#"SELECT
            (SELECT coalesce(sum(octet_length(data::text)+octet_length(subject::text)+512),0)::bigint FROM evidence_facts WHERE project_id=$1 AND data->>'superseded' IS DISTINCT FROM 'true')
            +(SELECT coalesce(sum(octet_length(r.definition::text)),0)::bigint FROM interface_documents d JOIN interface_environment_current c ON c.interface_id=d.id JOIN interface_observed_revisions r ON r.id=c.current_revision_id WHERE d.project_id=$1)
            +(SELECT coalesce(sum(octet_length(coalesce(context::text,''))+768),0)::bigint FROM capture_events e WHERE {SOURCE_FILTER})
            +(SELECT coalesce(sum(octet_length(proposed_definition::text)+512),0)::bigint FROM interface_observed_differences WHERE interface_id IN (SELECT id FROM interface_documents WHERE project_id=$1))"#)))
            .bind(id(project)).fetch_one(&mut **tx).await.map_err(db)?;
        if estimated > self.max_snapshot_bytes as i64 {
            return Err(invalid("SNAPSHOT_TOO_LARGE"));
        }
        let query = format!(
            r#"WITH materials AS ({}) SELECT jsonb_build_object(
         'interfaces',(SELECT coalesce(jsonb_agg(v ORDER BY v->>'method',v->>'path'),'[]') FROM (
          SELECT jsonb_build_object('interface_id',d.id,'method',d.method,'path',d.path,'environments',jsonb_agg(jsonb_build_object('environment_id',e.id,'environment_name',e.name,'revision_id',r.id,'definition',r.definition) ORDER BY e.id)) v
          FROM interface_documents d JOIN interface_environment_current c ON c.interface_id=d.id JOIN interface_observed_revisions r ON r.id=c.current_revision_id JOIN environments e ON e.id=c.environment_id WHERE d.project_id=$1 GROUP BY d.id) q),
         'observations',(SELECT coalesce(jsonb_agg(jsonb_build_object('record',value,'reconstructed_definition',null) ORDER BY value->>'ingestion_id'),'[]') FROM materials),
         'facts',(SELECT coalesce(jsonb_agg(to_jsonb(f)-'key_hash'-'subject_hash' || jsonb_build_object('samples',coalesce((SELECT jsonb_agg(event_id ORDER BY event_id) FROM evidence_samples s WHERE s.fact_id=f.id),'[]')) ORDER BY f.id),'[]') FROM evidence_facts f WHERE f.project_id=$1 AND f.data->>'superseded' IS DISTINCT FROM 'true'),
         'sources',(SELECT coalesce(jsonb_agg(jsonb_build_object('event_id',e.id,'project_id',e.project_id,'environment_id',e.environment_id,'actor_id',e.actor_id,'producer_id',e.producer_id,'record_id',e.record_id,'ingestion_id',e.ingestion_id,'kind',e.kind,'captured_at',e.captured_at,'context',e.context,'raw_hash',e.raw_hash,'evidence_status',e.evidence_status,'evidence_coverage',e.evidence_coverage) ORDER BY e.id),'[]') FROM capture_events e WHERE {SOURCE_FILTER}),
         'gaps',jsonb_build_object(
          'structure_pending',(SELECT count(*) FROM ingestion_inbox WHERE project_id=$1 AND status IN ('pending','processing')),
          'structure_failed',(SELECT count(*) FROM ingestion_inbox WHERE project_id=$1 AND status='failed'),
          'evidence_pending',(SELECT count(*) FROM capture_events WHERE project_id=$1 AND evidence_status IN ('pending','processing')),
          'evidence_failed',(SELECT count(*) FROM capture_events WHERE project_id=$1 AND evidence_status='failed'),
          'legacy_facts',(SELECT count(*) FROM evidence_facts WHERE project_id=$1 AND data->>'superseded' IS DISTINCT FROM 'true' AND (data->>'evidence_rule_version' IS DISTINCT FROM 'evidence-2' OR data->>'needs_reassessment'='true')),
          'unassessed_observations',(SELECT count(*) FROM materials WHERE value->'assessment'='null'::jsonb),
          'raw_unavailable',(SELECT count(*) FROM capture_events e WHERE {SOURCE_FILTER} AND raw_hash IS NULL)))"#,
            crate::documents::assessments::MATERIAL
        );
        let bundle: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .bind(id(project))
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?;
        let mut interfaces: Vec<CatalogInterface> = decode(bundle["interfaces"].clone())?;
        for interface in &mut interfaces {
            for environment in &mut interface.environments {
                environment.definition =
                    nexofolio_knowledge::compact_definition(environment.definition.take());
            }
        }
        let facts: Vec<EvidenceFact> = decode(bundle["facts"].clone())?;
        let mut inputs: SnapshotInputs = decode(
            serde_json::json!({"observations":bundle["observations"],"sources":bundle["sources"],"gaps":bundle["gaps"]}),
        )?;
        let needed: std::collections::HashSet<_> = facts
            .iter()
            .flat_map(|f| nexofolio_rebuild::evidence_field_refs(&f.subject))
            .filter_map(|f| f.observation)
            .filter(|s| s.project_id == project)
            .map(|s| s.ingestion_id)
            .collect();
        let reconstruct: Vec<_> = inputs
            .observations
            .iter()
            .filter(|m| {
                needed.contains(&m.record.ingestion_id)
                    && m.record.incoming_definition.as_ref().is_none_or(|d| {
                        !matches!(
                            d["extractor_version"].as_str(),
                            Some("observed-http-2" | "observed-http-3")
                        )
                    })
            })
            .map(|m| m.record.ingestion_id)
            .collect();
        // Raw records are immutable after admission (except retention); the held project evidence lock protects retention.
        let bytes:i64=sqlx::query_scalar("SELECT coalesce(sum(octet_length(raw_record::text)),0)::bigint FROM ingestion_inbox WHERE project_id=$1 AND id=ANY($2)").bind(id(project)).bind(&reconstruct).fetch_one(&mut **tx).await.map_err(db)?;
        if bytes > self.max_snapshot_bytes as i64 {
            return Err(invalid("SNAPSHOT_SOURCE_TOO_LARGE"));
        }
        for row in sqlx::query("SELECT id,raw_record,path_identity,identity_key FROM ingestion_inbox WHERE project_id=$1 AND id=ANY($2)").bind(id(project)).bind(&reconstruct).fetch_all(&mut **tx).await.map_err(db)? {
            let raw:Option<Value>=row.get("raw_record");
            if let Some(raw)=raw {
                let mut definition=nexofolio_knowledge::extract_observed(&raw)?;
                if let Some(path)=row.get::<Option<Value>,_>("path_identity") {nexofolio_knowledge::apply_observed_path(&mut definition,&decode(path)?)?;}
                if format!("{} {}",definition.method,definition.path)!=row.get::<String,_>("identity_key") {
                    *inputs.gaps.entry("reconstruction_identity_mismatch".into()).or_default()+=1;
                    continue;
                }
                inputs.observations.iter_mut().find(|m|m.record.ingestion_id==row.get::<Uuid,_>("id")).expect("selected material").reconstructed_definition=Some(encode(&definition));
            } else {*inputs.gaps.entry("observation_raw_unavailable".into()).or_default()+=1;}
        }
        inputs.gaps.insert(
            "extraction_incomplete_sources".into(),
            inputs
                .sources
                .iter()
                .filter(|s| {
                    s.kind == "http_exchange"
                        && s.evidence_coverage.as_ref().is_none_or(|c| {
                            c["request_complete"] != true || c["response_complete"] != true
                        })
                })
                .count() as u64,
        );
        Ok((interfaces, facts, inputs))
    }
}

//! Isolated snapshot characterization. No engine/model calls and no publication.
use nexofolio_application::{MaintenanceSources, MaintenanceStore};
use nexofolio_contracts::*;
use nexofolio_infrastructure::{
    FileBlobStore, Postgres, PostgresCaptureStore, PostgresMaintenance,
};
use nexofolio_rebuild::{field_maintenance_hints, review_segments, validate_segment};
use serde_json::{Value, json};
use sqlx::Row;
use std::{collections::HashSet, sync::Arc, time::Duration};
use uuid::Uuid;
pub fn run(input: &Value) -> Value {
    let url = input["database_url"].as_str().unwrap();
    assert!(input["isolated_database"] == true && url.contains("/stage2_audit"));
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let db=Postgres::new(&Secret::new(url),5,Duration::from_secs(2)).unwrap();db.migrate().await.unwrap();
        let sql=sqlx::PgPool::connect(url).await.unwrap();
        let row=sqlx::query("SELECT a.user_id,a.project_id FROM user_project_access a WHERE EXISTS(SELECT 1 FROM interface_documents d WHERE d.project_id=a.project_id) LIMIT 1").fetch_one(&sql).await.unwrap();
        let user:UserId=row.get::<Uuid,_>("user_id").to_string().parse().unwrap();let project:ProjectId=row.get::<Uuid,_>("project_id").to_string().parse().unwrap();
        let store=PostgresMaintenance::new(db.clone());
        let request=StartMaintenance{request_id:Uuid::new_v4()};
        let initial=store.start(user,project,&request).await;
        let initial_error=initial.as_ref().err().map(|e|format!("{e:?}"));
        // Test setup only: supply the missing system directory state to inspect downstream behavior.
        sqlx::query("INSERT INTO project_catalogs(project_id) VALUES($1) ON CONFLICT DO NOTHING").bind(row.get::<Uuid,_>("project_id")).execute(&sql).await.unwrap();
        let legacy=store.start(user,project,&request).await.unwrap();
        let legacy_snapshot=store.snapshot(user,project,legacy.id).await.unwrap();
        let legacy_count=legacy_snapshot.facts.iter().filter(|f|f.data["needs_reassessment"]==true).count();
        // Rebuild only the isolated derivative evidence, keeping original capture and interface definitions.
        sqlx::raw_sql("TRUNCATE evidence_samples,evidence_relation_pairs,evidence_relation_values,evidence_ui_pairs,evidence_sample_groups,evidence_ui_bindings,evidence_value_index,evidence_facts; UPDATE capture_events SET evidence_status='pending',attempts=0,retry_at=clock_timestamp(),lease_until=NULL; UPDATE capture_backlog b SET pending=(SELECT count(*) FROM capture_events e WHERE e.project_id=b.project_id)").execute(&sql).await.unwrap();
        let capture=PostgresCaptureStore::new(db.clone(),Arc::new(FileBlobStore::new(input["blob_root"].as_str().unwrap().into())));
        let mut processed=0;while capture.process_evidence_one().await.unwrap(){processed+=1;assert!(processed<=500);}
        // Populate the existing Stage 1 column from the real stored pending definition, without adopting it.
        let d=sqlx::query("SELECT o.ingestion_id,r.definition,f.proposed_definition FROM interface_observations o JOIN interface_observed_revisions r ON r.id=o.compared_revision_id JOIN interface_observed_differences f ON f.id=o.difference_id LIMIT 1").fetch_one(&sql).await.unwrap();
        let assessment=nexofolio_knowledge::assess_definition(Some(&d.get::<Value,_>("definition")),&d.get::<Value,_>("proposed_definition"));
        sqlx::query("UPDATE interface_observations SET assessment=$2 WHERE ingestion_id=$1").bind(d.get::<Uuid,_>("ingestion_id")).bind(serde_json::to_value(&assessment).unwrap()).execute(&sql).await.unwrap();
        let req=StartMaintenance{request_id:Uuid::new_v4()};let run=store.start(user,project,&req).await.unwrap();
        assert_eq!(run.id,store.start(user,project,&req).await.unwrap().id);
        let snapshot=store.snapshot(user,project,run.id).await.unwrap();let frozen=serde_json::to_value(&snapshot).unwrap();
        let unadopted:Vec<_>=snapshot.facts.iter().filter(|f|f.subject.get("observation").is_some()).collect();
        let indexed_refs:HashSet<_>=snapshot.fields.iter().map(|f|serde_json::to_value(&f.reference).unwrap().to_string()).collect();
        let unadopted_refs:HashSet<_>=unadopted.iter().map(|f|f.subject.to_string()).collect();
        let mut segment_report=json!({});
        let mut rejects_omitted=false;let mut rejects_duplicate=false;
        match review_segments(&snapshot,65536) {
            Ok(segments)=>{
                let mut covered=HashSet::new();let mut units=0;
                for segment in &segments {
                    let review=SegmentReview{segment_id:segment.id.clone(),assessments:segment.units.iter().map(|u|FieldAssessment{unit_id:u.id.clone(),field_id:u.field_id.clone(),disposition:"keep".into(),note:"Synthetic coverage check, no model read".into(),evidence:u.context["evidence_summary"]["representatives"].as_array().into_iter().flatten().filter(|r|matches!(r["fact_kind"].as_str(),Some("parameter_link_candidate"|"parameter_link_counterexample"))).map(|r|serde_json::from_value(r["reference"].clone()).unwrap()).collect()}).collect(),summary:"Synthetic test only".into()};
                    validate_segment(segment,&review).unwrap();
                    if !review.assessments.is_empty() {
                        let mut incomplete=review.clone();incomplete.assessments.pop();rejects_omitted |= validate_segment(segment,&incomplete).is_err();
                        let mut repeated=review.clone();repeated.assessments.push(review.assessments[0].clone());rejects_duplicate |= validate_segment(segment,&repeated).is_err();
                    }
                    for u in &segment.units {covered.insert(u.field_id.clone());units+=1;}
                }
                segment_report=json!({"segments":segments.len(),"units":units,"fields_covered":covered.len(),"declared_fields":snapshot.fields.len(),"all_declared_fields_covered":covered.len()==snapshot.fields.len(),"rejects_omitted_unit":rejects_omitted,"rejects_duplicate_unit":rejects_duplicate,"model_read":false});
            },Err(e)=>{segment_report["error"]=json!(format!("{e:?}"));}
        }
        let event=*snapshot.facts.iter().find_map(|f|f.samples.first()).unwrap();
        let raw=MaintenanceSources::observation(&capture,snapshot.inputs.as_ref().unwrap().sources.iter().find(|s|s.event_id==event).unwrap()).await.unwrap();
        let other:Uuid=sqlx::query_scalar("SELECT id FROM projects WHERE id<>$1 LIMIT 1").bind(row.get::<Uuid,_>("project_id")).fetch_one(&sql).await.unwrap();
        let other_project=other.to_string().parse().unwrap();
        let cross_project_read_rejected=store.snapshot(user,other_project,run.id).await.is_err();
        let mut hints_probe=serde_json::to_value(field_maintenance_hints(&legacy_snapshot)).unwrap().to_string();
        let legacy_navigation_exposes_reassessment=hints_probe.contains("needs_reassessment");hints_probe.clear();
        let samples:HashSet<_>=snapshot.facts.iter().flat_map(|f|f.samples.iter()).collect();
        let pins:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_pins WHERE owner_kind='maintenance_snapshot' AND owner_id=$1").bind(run.id).fetch_one(&sql).await.unwrap();
        let before_pending=snapshot.pending_observations;
        // Derived lifecycle probe: structure processing pending while evidence is already completed.
        let ingestion:Uuid=sqlx::query_scalar("SELECT id FROM ingestion_inbox LIMIT 1").fetch_one(&sql).await.unwrap();
        sqlx::query("UPDATE ingestion_inbox SET status='pending' WHERE id=$1").bind(ingestion).execute(&sql).await.unwrap();
        let pending_run=store.start(user,project,&StartMaintenance{request_id:Uuid::new_v4()}).await.unwrap();
        let pending_snapshot=store.snapshot(user,project,pending_run.id).await.unwrap();
        sqlx::query("UPDATE ingestion_inbox SET status='completed' WHERE id=$1").bind(ingestion).execute(&sql).await.unwrap();
        sqlx::query("UPDATE evidence_facts SET data=data||'{\"stage3_after_snapshot_probe\":true}'::jsonb WHERE id=$1").bind(snapshot.facts[0].id).execute(&sql).await.unwrap();
        let reread=store.snapshot(user,project,run.id).await.unwrap();assert_eq!(frozen,serde_json::to_value(reread).unwrap());
        let limited=PostgresMaintenance::with_snapshot_limit(db.clone(),1);
        let budget_rejected=limited.start(user,project,&StartMaintenance{request_id:Uuid::new_v4()}).await.is_err();
        assert!(snapshot.inputs.as_ref().unwrap().observations.iter().any(|m|m.record.assessment.is_some()));
        assert!(unadopted_refs.iter().all(|r|indexed_refs.contains(r)));
        assert_eq!(pending_snapshot.inputs.as_ref().unwrap().gaps["structure_pending"],1);
        assert!(raw.get("captured_at").is_some());
        let findings=json!({"processed":processed,"catalog_start_error_before_test_setup":initial_error,"legacy_snapshot":{"facts":legacy_snapshot.facts.len(),"legacy_needing_reassessment":legacy_count,"navigation_exposes_reassessment":legacy_navigation_exposes_reassessment,"pending_observations":legacy_snapshot.pending_observations},"current_snapshot":{"interfaces":snapshot.interfaces.len(),"fields":snapshot.fields.len(),"facts":snapshot.facts.len(),"unadopted_value_facts":unadopted.len(),"unadopted_distinct_refs":unadopted_refs.len(),"unadopted_refs_indexed":unadopted_refs.iter().filter(|r|indexed_refs.contains(*r)).count(),"input_observations":snapshot.inputs.as_ref().map(|i|i.observations.len()),"frozen_sources":snapshot.inputs.as_ref().map(|i|i.sources.len()),"input_gaps":snapshot.inputs.as_ref().map(|i|&i.gaps),"field_knowledge_hints":field_maintenance_hints(&snapshot).len(),"has_structural_assessment_collection":snapshot.inputs.as_ref().is_some_and(|i|i.observations.iter().any(|m|m.record.assessment.is_some())),"has_observation_structure_collection":snapshot.inputs.as_ref().is_some_and(|i|!i.observations.is_empty()),"has_capture_coverage_collection":snapshot.inputs.as_ref().is_some_and(|i|i.sources.iter().any(|s|s.evidence_coverage.is_some())),"pending_observations":before_pending},"declared_review":segment_report,"snapshot_immutable_after_fact_update":true,"request_id_idempotent":true,"sample_pins":pins,"sample_events":samples.len(),"cross_project_read_rejected":cross_project_read_rejected,"observation_reference_read_supported":nexofolio_rebuild::snapshot_contains_reference(&snapshot,&KnowledgeEvidenceRef{kind:"observation".into(),id:d.get::<Uuid,_>("ingestion_id").to_string()}),"readback_has_captured_at":raw.get("captured_at").is_some(),"structure_pending_probe_reported_count":pending_snapshot.inputs.as_ref().and_then(|i|i.gaps.get("structure_pending")),"budget_rejected":budget_rejected,"model_calls":0,"published":false});
        sql.close().await;db.close().await;findings
    })
}

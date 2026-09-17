//! Isolated preflight and explicitly selected fixture/provider trial. Never publishes.
use nexofolio_application::{
    MaintenanceBudget, MaintenanceEngine, MaintenanceStore, prepare_maintenance_review,
};
use nexofolio_contracts::*;
use nexofolio_infrastructure::{
    ChatMaintenanceModel, FileBlobStore, Postgres, PostgresCaptureStore, PostgresMaintenance,
};
use nexofolio_rebuild::MaintenanceModel;
use serde_json::{Value, json};
use sqlx::Row;
use std::{path::Path, sync::Arc, time::Duration};
use uuid::Uuid;
pub fn run(input: &Value) -> Value {
    let url = input["database_url"].as_str().unwrap();
    assert!(input["isolated_database"] == true && url.contains("/stage2_audit"));
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let db=Postgres::new(&Secret::new(url),5,Duration::from_secs(2)).unwrap();db.migrate().await.unwrap();
        let sql=sqlx::PgPool::connect(url).await.unwrap();
        let out=Path::new(input["output_dir"].as_str().unwrap());
        let capture=Arc::new(PostgresCaptureStore::new(db.clone(),Arc::new(FileBlobStore::new(input["blob_root"].as_str().unwrap().into()))));
        let store=Arc::new(PostgresMaintenance::new(db.clone()));
        let (user,project,run,processed)=if let Some(resume)=input["resume_run"].as_str() {
            let id:Uuid=resume.parse().unwrap();
            let row=sqlx::query("SELECT actor_id,project_id FROM maintenance_runs WHERE id=$1 AND status='incomplete' AND error_code='PLANNING_CONTEXT_REPAIR' AND candidate IS NULL").bind(id).fetch_one(&sql).await.unwrap();
            let user:UserId=row.get::<Uuid,_>("actor_id").to_string().parse().unwrap();
            let project:ProjectId=row.get::<Uuid,_>("project_id").to_string().parse().unwrap();
            // The immutable snapshot, model settings and review rules are unchanged.
            // claim() still checks the existing settings fingerprint; never clear it.
            sqlx::query("UPDATE maintenance_runs SET status='pending',error_code=NULL,lease_until=NULL WHERE id=$1").bind(id).execute(&sql).await.unwrap();
            (user,project,store.get(user,project,id).await.unwrap(),0)
        } else {
        let mut processed=0;
        if input["preserve_evidence"]!=true {
        sqlx::raw_sql("TRUNCATE evidence_samples,evidence_relation_pairs,evidence_relation_values,evidence_ui_pairs,evidence_sample_groups,evidence_ui_bindings,evidence_value_index,evidence_facts; UPDATE capture_events SET evidence_status='pending',attempts=0,retry_at=clock_timestamp(),lease_until=NULL; UPDATE capture_backlog b SET pending=(SELECT count(*) FROM capture_events e WHERE e.project_id=b.project_id)").execute(&sql).await.unwrap();
        while capture.process_evidence_one().await.unwrap(){processed+=1;assert!(processed<=500);}
        }
        let row=sqlx::query("SELECT a.user_id,a.project_id FROM user_project_access a WHERE EXISTS(SELECT 1 FROM interface_documents d WHERE d.project_id=a.project_id) LIMIT 1").fetch_one(&sql).await.unwrap();
        let user:UserId=row.get::<Uuid,_>("user_id").to_string().parse().unwrap();let project:ProjectId=row.get::<Uuid,_>("project_id").to_string().parse().unwrap();

        let run=store.start(user,project,&StartMaintenance{request_id:Uuid::new_v4()}).await.unwrap();
            (user,project,run,processed)
        };
        let snapshot=store.snapshot(user,project,run.id).await.unwrap();
        std::fs::write(out.join("snapshot.private.json"),serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let context=input["context_tokens"].as_u64().unwrap() as usize;
        let max_calls=input["max_calls"].as_u64().unwrap() as u32;
        let budget=MaintenanceBudget {context_tokens:context,max_calls,retries:2};
        let key=if input["execute_model"]==true {std::fs::read_to_string(input["model_key_file"].as_str().unwrap()).unwrap()} else {"offline-unused".into()};
        let model=Arc::new(ChatMaintenanceModel::with_timeout(input["model_base_url"].as_str().unwrap_or("http://127.0.0.1:9/v1"),Secret::new(key.trim()),input["model"].as_str().unwrap().into(),false,Duration::from_millis(input["model_timeout_ms"].as_u64().unwrap_or(240000))).unwrap());
        let active=Arc::new(std::sync::atomic::AtomicUsize::new(0));let peak=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let model: Arc<dyn MaintenanceModel> = if input["execute_fixture"]==true {Arc::new(FixtureModel {inner:model,active:active.clone(),peak:peak.clone(),fail_first:input["fixture_fail_first"]==true,read_all:input["fixture_read_all"]==true,interfaces:snapshot.interfaces.iter().map(|i|i.interface_id.to_string()).collect(),received:std::sync::Mutex::new(std::collections::HashSet::new()),requested:std::sync::atomic::AtomicBool::new(false),plan:fixture_plan(&snapshot)})} else if input["review_low"]==true {Arc::new(LowReviewModel(model))} else {model};
        let segments=prepare_maintenance_review(&snapshot,model.as_ref(),&budget);
        let before_calls=run.model_calls;
        let before_checkpoints:i64=sqlx::query_scalar("SELECT count(*) FROM maintenance_checkpoints WHERE run_id=$1 AND value->'review'<>'null'::jsonb").bind(run.id).fetch_one(&sql).await.unwrap();
        let mut report=json!({"run_id":run.id,"project_id":project,"processed":processed,"interfaces":snapshot.interfaces.len(),"fields":snapshot.fields.len(),"facts":snapshot.facts.len(),"context_tokens":context,"max_calls":max_calls,"model":input["model"],"snapshot_bytes":serde_json::to_vec(&snapshot).unwrap().len(),"model_executed":false});
        match segments {
            Ok((segments,estimate))=>{report["review_segments"]=json!(segments.len());report["total_reserved_calls"]=json!(estimate.total_calls());report["preflight_ok"]=json!(estimate.total_calls()<=max_calls as usize);report["estimate"]=json!(estimate);}
            Err(e)=>{report["preflight_ok"]=json!(false);report["preflight_error"]=json!(format!("{e:?}"));}
        }
        std::fs::write(out.join("preflight.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        if (input["execute_model"]==true || input["execute_fixture"]==true) && report["preflight_ok"]==true {
            std::fs::OpenOptions::new().write(true).create_new(true).open(out.join("execution-started")).expect("do not repeat real trial in this directory");
            let engine=MaintenanceEngine {store:store.clone(),model,sources:capture,budget};
            let seconds=input["wall_timeout_seconds"].as_u64().unwrap_or(480).min(7200);
            let result=tokio::time::timeout(Duration::from_secs(seconds),engine.tick()).await;
            report["model_execution_started"]=json!(true);
            report["execution_result"]=json!(format!("{result:?}"));
            if result.is_err() {
                sqlx::query("UPDATE maintenance_runs SET status='incomplete',phase='trial_timeout',error_code='TRIAL_WALL_TIME_LIMIT',generation=generation+1,lease_until=NULL WHERE id=$1 AND status='running'").bind(run.id).execute(&sql).await.unwrap();
            }
        }
        let finished=store.get(user,project,run.id).await.unwrap();
        report["run"]=serde_json::to_value(&finished).unwrap();
        report["status"]=json!(finished.status);report["coverage"]=json!(finished.coverage);report["error_code"]=json!(finished.error_code);
        std::fs::write(out.join("result.private.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        let calls:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'created_at',created_at,'finished_at',finished_at,'request',request,'response',response) FROM maintenance_calls WHERE run_id=$1 ORDER BY created_at,id").bind(run.id).fetch_all(&sql).await.unwrap();
        for (n,call) in calls.iter().enumerate(){std::fs::write(out.join(format!("call-{n:03}.private.json")),serde_json::to_vec_pretty(call).unwrap()).unwrap();}
        report["captured_calls"]=json!(calls.len());
        report["resumed"]=json!(input["resume_run"].is_string());
        report["previous_calls"]=json!(before_calls);report["previous_review_checkpoints"]=json!(before_checkpoints);
        if input["execute_fixture"]==true {
            report["peak_review_concurrency"]=json!(peak.load(std::sync::atomic::Ordering::SeqCst));
            assert!(peak.load(std::sync::atomic::Ordering::SeqCst)<=4);
            if input["fixture_fail_first"]==true {assert!(finished.candidate.is_none() && finished.coverage.completed_segments==3);}
            else {assert!(finished.coverage.complete && peak.load(std::sync::atomic::Ordering::SeqCst)==4 && finished.candidate.is_some());}
        }
        report["real_provider"]=json!(input["execute_model"]==true && !calls.is_empty());
        report["model_executed"]=json!(!calls.is_empty());
        std::fs::write(out.join("result.private.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        report.as_object_mut().unwrap().remove("run");
        sql.close().await;db.close().await;report
    })
}

/// Entire fixed snapshot through the real engine; synthetic conclusions only.
struct FixtureModel {
    inner: Arc<ChatMaintenanceModel>,
    active: Arc<std::sync::atomic::AtomicUsize>,
    peak: Arc<std::sync::atomic::AtomicUsize>,
    fail_first: bool,
    read_all: bool,
    interfaces: Vec<String>,
    received: std::sync::Mutex<std::collections::HashSet<String>>,
    requested: std::sync::atomic::AtomicBool,
    plan: Value,
}
fn fixture_plan(snapshot: &KnowledgeSnapshot) -> Value {
    let field = snapshot
        .fields
        .iter()
        .find(|f| {
            f.reference.adopted().is_some()
                && f.reference.location == "response.body"
                && !f.reference.path.is_empty()
        })
        .unwrap();
    json!({"strategy":"keep","reason":"受控读取测试","expected_benefit":"仅验证全量读取和一项语义候选合同，不代表实际模型收益","actions":[{"kind":"upsert_annotation","reason":"测试候选","annotation":{"id":Uuid::new_v4(),"target":{"kind":"field","field":field.reference.adopted().unwrap()},"value":{"kind":"description","text":"受控测试说明，不是业务文档"},"evidence":[{"kind":"field","id":field.id}],"verification":"needs_review","note":"fixture only"}}]})
}
#[async_trait::async_trait]
impl MaintenanceModel for FixtureModel {
    fn identity(&self) -> Value {
        json!({"fixture":"stage4-full-engine-no-semantic-claim"})
    }
    fn prepare(&self, phase: &str, input: Value, output: usize) -> Result<Value> {
        self.inner.prepare(phase, input, output)
    }
    async fn invoke(&self, request: &Value) -> Result<Value> {
        let input: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        let segment = &input["input"]["segment"];
        if segment.is_object() {
            use std::sync::atomic::Ordering::SeqCst;
            let active = self.active.fetch_add(1, SeqCst) + 1;
            self.peak.fetch_max(active, SeqCst);
            tokio::time::sleep(Duration::from_millis(40)).await;
            self.active.fetch_sub(1, SeqCst);
            if self.fail_first && segment["id"] == "segment-0" {
                return Err(Error::Unavailable {
                    component: "fixture_first_segment",
                });
            }
            return Ok(
                json!({"type":"review","review":{"segment_id":segment["id"],"summary":"受控测试保持现状；不代表真实模型判断。","assessments":segment["units"].as_array().unwrap().iter().map(|u|json!({"unit_id":u["id"],"field_id":u["field_id"],"disposition":"keep","note":"fixture","evidence":u["context"]["evidence_summary"]["representatives"].as_array().into_iter().flatten().filter(|r|matches!(r["fact_kind"].as_str(),Some("parameter_link_candidate"|"parameter_link_counterexample"))).map(|r|r["reference"].clone()).collect::<Vec<_>>()})).collect::<Vec<_>>()}}),
            );
        }
        if self.read_all {
            if input["input"]["reference"]["kind"] == "interface"
                && input["input"]["part"] == input["input"]["parts"]
            {
                self.received.lock().unwrap().insert(
                    input["input"]["reference"]["id"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
            for item in input["input"]["pending_originals"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if item["reference"]["kind"] == "interface" {
                    assert!(!item["originals"].as_array().unwrap().is_empty());
                    self.received
                        .lock()
                        .unwrap()
                        .insert(item["reference"]["id"].as_str().unwrap().to_owned());
                }
            }
            if input["input"]["coverage"].is_object() {
                if !self
                    .requested
                    .swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    return Ok(
                        json!({"type":"read","requests":[{"kind":"interface","ids":self.interfaces},{"kind":"summary","ids":["snapshot-index"]}]}),
                    );
                }
                if self.received.lock().unwrap().len() != self.interfaces.len() {
                    // Repeated navigation requests must not block still-pending originals.
                    return Ok(
                        json!({"type":"read","requests":[{"kind":"summary","ids":["snapshot-index"]}]}),
                    );
                }
                if input["input"]["original_delivery"] == true {
                    return Ok(json!({"type":"read","requests":[]}));
                }
                if input["input"]["original_delivery"] == false {
                    assert!(input["input"]["current_catalog"]["nodes"].is_array());
                    assert!(input["input"]["all_interface_index"].is_array());
                    assert!(input["input"]["global_index"].is_array());
                }
                return Ok(json!({"type":"plan","plan":self.plan}));
            }
        }
        if input["input"]["coverage"].is_object() {
            Ok(
                json!({"type":"plan","plan":{"strategy":"keep","reason":"隔离受控测试","expected_benefit":"验证全量执行与覆盖，不证明语义收益","actions":[]}}),
            )
        } else {
            Ok(json!({"type":"summary","summary":"受控测试索引已汇总；无业务判断。"}))
        }
    }
}

/// Trial-only setting: only the review phase has been tested with low effort.
struct LowReviewModel(Arc<ChatMaintenanceModel>);
#[async_trait::async_trait]
impl MaintenanceModel for LowReviewModel {
    fn identity(&self) -> Value {
        json!({"model":self.0.identity(),"review_effort":"low"})
    }
    fn prepare(&self, phase: &str, input: Value, output: usize) -> Result<Value> {
        let mut request = self.0.prepare(phase, input, output)?;
        if phase == "review" {
            request["thinking"] = json!({"type":"enabled"});
            request["reasoning_effort"] = json!("low");
        }
        Ok(request)
    }
    async fn invoke(&self, request: &Value) -> Result<Value> {
        self.0.invoke(request).await
    }
    async fn invoke_tracked(&self, request: &Value) -> Result<nexofolio_rebuild::ModelInvocation> {
        self.0.invoke_tracked(request).await
    }
}

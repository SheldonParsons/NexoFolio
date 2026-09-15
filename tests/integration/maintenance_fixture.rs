use nexofolio_application::CatalogActivationStore;
use nexofolio_application::{
    KnowledgeActivationStore, MaintenanceBudget, MaintenanceEngine, MaintenanceStore,
};
use nexofolio_contracts::*;
use nexofolio_infrastructure::{
    Postgres, PostgresCaptureStore, PostgresMaintenance, PostgresOfficialCatalog,
};
use nexofolio_knowledge::OfficialCatalogReader;
use nexofolio_rebuild::MaintenanceModel;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
struct Model {
    plan: MaintenancePlan,
}
#[async_trait::async_trait]
impl MaintenanceModel for Model {
    fn identity(&self) -> Value {
        json!({"fixture":"maintenance-controlled"})
    }
    fn prepare(&self, phase: &str, input: Value, _: usize) -> Result<Value> {
        Ok(json!({"phase":phase,"input":input}))
    }
    async fn invoke(&self, request: &Value) -> Result<Value> {
        let input = &request["input"]["input"];
        match request["phase"].as_str().unwrap() {
            "review" => {
                let segment = &input["segment"];
                Ok(
                    json!({"type":"review","review":{"segment_id":segment["id"],"summary":"合成字段已审阅","assessments":segment["units"].as_array().unwrap().iter().map(|u|json!({"unit_id":u["id"],"field_id":u["field_id"],"disposition":"keep","note":"test observed field","evidence":[]})).collect::<Vec<_>>()}}),
                )
            }
            "plan" => Ok(json!({"type":"plan","plan":self.plan})),
            _ => Ok(json!({"type":"summary","summary":"合成原文已回读；不宣称真实LLM语义能力"})),
        }
    }
}
struct InvalidPhaseModel {
    inner: Model,
    phase: &'static str,
}
#[async_trait::async_trait]
impl MaintenanceModel for InvalidPhaseModel {
    fn identity(&self) -> Value {
        json!({"fixture":"invalid_phase","phase":self.phase})
    }
    fn prepare(&self, phase: &str, input: Value, output: usize) -> Result<Value> {
        self.inner.prepare(phase, input, output)
    }
    async fn invoke(&self, request: &Value) -> Result<Value> {
        if request["phase"] == self.phase {
            return Ok(
                json!({"type":"summary","summary":if self.phase=="summary"{"x".repeat(20000)}else{String::new()}}),
            );
        }
        let mut reply = self.inner.invoke(request).await?;
        if self.phase == "summary" && request["phase"] == "review" {
            // Explicitly force summary compaction; do not depend on the old six-unit shard limit.
            for assessment in reply["review"]["assessments"].as_array_mut().unwrap() {
                assessment["note"] = json!("detailed synthetic finding ".repeat(24));
            }
        }
        Ok(reply)
    }
}
struct NavigationModel {
    inner: Model,
    requested: std::sync::atomic::AtomicBool,
    saw_original: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl MaintenanceModel for NavigationModel {
    fn identity(&self) -> Value {
        json!({"fixture":"navigation"})
    }
    fn prepare(&self, phase: &str, input: Value, output: usize) -> Result<Value> {
        self.inner.prepare(phase, input, output)
    }
    async fn invoke(&self, request: &Value) -> Result<Value> {
        use std::sync::atomic::Ordering;
        if request["phase"] == "plan" {
            if !self.requested.swap(true, Ordering::SeqCst) {
                return Ok(
                    json!({"type":"read","requests":[{"kind":"summary","ids":["snapshot-index"]}]}),
                );
            }
            if let Some(index) =
                request["input"]["input"]["exact_read_sources"].get("navigation:snapshot-index")
            {
                assert!(index["fields"].is_array() || index["complete_index"] == true);
                self.saw_original.store(true, Ordering::SeqCst);
            }
        }
        self.inner.invoke(request).await
    }
}
struct RepairModel {
    inner: Model,
    bad_plan_sent: std::sync::atomic::AtomicBool,
    saw_feedback: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl MaintenanceModel for RepairModel {
    fn identity(&self) -> Value {
        json!({"fixture":"repair"})
    }
    fn prepare(&self, phase: &str, input: Value, output: usize) -> Result<Value> {
        self.inner.prepare(phase, input, output)
    }
    async fn invoke(&self, request: &Value) -> Result<Value> {
        use std::sync::atomic::Ordering;
        if request["phase"] == "plan" {
            if !self.bad_plan_sent.swap(true, Ordering::SeqCst) {
                let mut plan = self.inner.plan.clone();
                if let MaintenanceAction::SetDirectory { node, .. } = &mut plan.actions[0] {
                    node.name = "待分类".into();
                }
                return Ok(json!({"type":"plan","plan":plan}));
            }
            if request["input"]["input"]["validation_feedback"]["code"]
                == "INVALID_DIRECTORY_CHANGE"
            {
                self.saw_feedback.store(true, Ordering::SeqCst);
            }
        }
        self.inner.invoke(request).await
    }
}
struct PhaseConfusionModel {
    inner: Model,
    wrong_sent: std::sync::atomic::AtomicBool,
    saw_feedback: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl MaintenanceModel for PhaseConfusionModel {
    fn identity(&self) -> Value {
        json!({"fixture":"phase_confusion"})
    }
    fn prepare(&self, phase: &str, input: Value, output: usize) -> Result<Value> {
        self.inner.prepare(phase, input, output)
    }
    async fn invoke(&self, request: &Value) -> Result<Value> {
        use std::sync::atomic::Ordering;
        if request["phase"] == "plan" {
            if !self.wrong_sent.swap(true, Ordering::SeqCst) {
                return Ok(json!({"type":"summary","summary":"wrong phase"}));
            }
            if request["input"]["previous_error"]["current_phase"] == "plan" {
                self.saw_feedback.store(true, Ordering::SeqCst);
            }
        }
        self.inner.invoke(request).await
    }
}
pub async fn verify(
    db: &Postgres,
    sources: Arc<PostgresCaptureStore>,
    user: UserId,
    project: ProjectId,
    denied: ProjectId,
    sql: &sqlx::PgPool,
) {
    let store = Arc::new(PostgresMaintenance::new(db.clone()));
    let request = StartMaintenance {
        request_id: Uuid::new_v4(),
    };
    assert!(matches!(
        store.start(user, denied, &request).await,
        Err(Error::Forbidden)
    ));
    let run = store.start(user, project, &request).await.unwrap();
    assert!(matches!(run.status, MaintenanceStatus::Pending));
    assert_eq!(
        store.start(user, project, &request).await.unwrap().id,
        run.id
    );
    let snapshot = store.snapshot(user, project, run.id).await.unwrap();
    assert!(!snapshot.fields.is_empty());
    assert!(!snapshot.facts.is_empty());
    let f = snapshot
        .fields
        .iter()
        .find(|f| f.reference.location == "request.body" && !f.reference.path.is_empty())
        .unwrap();
    let node = PreviewNode {
        id: DirectoryId::new(),
        parent: None,
        name: "订单".into(),
        description: "合成维护测试".into(),
    };
    let reference = KnowledgeEvidenceRef {
        kind: "field".into(),
        id: f.id.clone(),
    };
    let annotation = AnnotationDraft {
        id: Uuid::new_v4(),
        target: SemanticTarget::Field {
            field: f.reference.clone(),
        },
        value: SemanticValue::Description {
            text: "用于订单详情查询的请求参数，含义仍需业务复核。".into(),
        },
        evidence: vec![reference.clone()],
        verification: Verification::Inferred,
        note: "synthetic contract test".into(),
    };
    let plan = MaintenancePlan {
        strategy: MaintenanceStrategy::Insert,
        reason: "合成目录和说明一起更新".into(),
        expected_benefit: "fixture".into(),
        actions: vec![
            MaintenanceAction::SetDirectory {
                node: node.clone(),
                reason: "fixture".into(),
                evidence: vec![reference.clone()],
            },
            MaintenanceAction::AssignInterface {
                interface_id: f.reference.interface_id,
                directory_id: Some(node.id),
                reason: "fixture".into(),
                evidence: vec![reference],
            },
            MaintenanceAction::UpsertAnnotation {
                annotation: Box::new(annotation),
                reason: "fixture".into(),
            },
        ],
    };
    let engine = MaintenanceEngine {
        store: store.clone(),
        sources: sources.clone(),
        model: Arc::new(Model { plan: plan.clone() }),
        budget: MaintenanceBudget::default(),
    };
    assert!(engine.tick().await.unwrap());
    let ready = store.get(user, project, run.id).await.unwrap();
    assert!(
        matches!(ready.status, MaintenanceStatus::Ready),
        "status {:?} error {:?}",
        ready.status,
        ready.error_code
    );
    assert!(ready.coverage.complete);
    assert_eq!(ready.coverage.reviewed_fields, snapshot.fields.len());
    assert!(ready.read_count > 0);
    let points = store
        .checkpoints(user, project, run.id, 1, 100)
        .await
        .unwrap();
    assert!(points.items.iter().any(|p| p.phase == "readback_complete"));
    assert!(
        points
            .items
            .iter()
            .any(|p| p.phase == "readback" && p.references.len() > 1),
        "multiple small originals share a successful model call"
    );
    let publish = PublishKnowledge {
        request_id: Uuid::new_v4(),
        expected_generation: 0,
    };
    let result = store
        .publish_knowledge(user, project, run.id, &publish)
        .await
        .unwrap();
    assert!(result.changed);
    assert_eq!(result.generation, 1);
    assert!(
        store
            .publish_knowledge(user, project, run.id, &publish)
            .await
            .unwrap()
            .replayed
    );
    let knowledge = store
        .interface_knowledge(
            user,
            project,
            f.reference.interface_id,
            f.reference.environment_id,
        )
        .await
        .unwrap();
    assert_eq!(knowledge.annotations.len(), 1);
    assert!(!knowledge.annotations[0].stale);
    let official = PostgresOfficialCatalog::new(db.clone());
    let catalog = official.current(user, project).await.unwrap();
    assert_eq!(catalog.source_run_id, Some(run.id));
    assert!(catalog.source_task_id.is_none());
    assert!(catalog.nodes.iter().any(|n| n.id == node.id));
    assert_eq!(
        official.versions(user, project, 1, 10).await.unwrap().items[0].source_run_id,
        Some(run.id)
    );
    // Old directory-only restore keeps semantics, creates coherent history, and shares generation.
    let legacy = official
        .restore(
            user,
            project,
            &RestoreCatalog {
                version_id: None,
                request_id: Uuid::new_v4(),
                expected_generation: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(legacy.generation, 2);
    assert_eq!(
        store
            .interface_knowledge(
                user,
                project,
                f.reference.interface_id,
                f.reference.environment_id
            )
            .await
            .unwrap()
            .annotations
            .len(),
        1
    );
    let restore = RestoreKnowledge {
        version_id: result.version_id,
        request_id: Uuid::new_v4(),
        expected_generation: 2,
    };
    let restored = store
        .restore_knowledge(user, project, &restore)
        .await
        .unwrap();
    assert_eq!(restored.generation, 3);
    assert!(
        store
            .restore_knowledge(user, project, &restore)
            .await
            .unwrap()
            .replayed
    );
    assert!(matches!(
        store
            .restore_knowledge(
                user,
                project,
                &RestoreKnowledge {
                    version_id: None,
                    request_id: Uuid::new_v4(),
                    expected_generation: 1
                }
            )
            .await,
        Err(Error::Conflict)
    ));
    let reset = store
        .restore_knowledge(
            user,
            project,
            &RestoreKnowledge {
                version_id: None,
                request_id: Uuid::new_v4(),
                expected_generation: 3,
            },
        )
        .await
        .unwrap();
    assert_eq!(reset.generation, 4);
    assert!(
        store
            .interface_knowledge(
                user,
                project,
                f.reference.interface_id,
                f.reference.environment_id
            )
            .await
            .unwrap()
            .annotations
            .is_empty()
    );
    assert_eq!(
        store
            .knowledge_versions(user, project, 1, 10)
            .await
            .unwrap()
            .generation,
        4
    );
    // Candidate against generation zero cannot overwrite a newer state with a new request ID.
    assert!(matches!(
        store
            .publish_knowledge(
                user,
                project,
                run.id,
                &PublishKnowledge {
                    request_id: Uuid::new_v4(),
                    expected_generation: 4
                }
            )
            .await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        store
            .snapshot(user, project, run.id)
            .await
            .unwrap()
            .base_generation,
        0
    );
    // Two different activations against one generation cannot both win.
    let versions = store
        .knowledge_versions(user, project, 1, 10)
        .await
        .unwrap();
    let targets: Vec<_> = versions.items.iter().map(|v| v.id).collect();
    assert!(targets.len() >= 2);
    let left = RestoreKnowledge {
        version_id: Some(targets[0]),
        request_id: Uuid::new_v4(),
        expected_generation: 4,
    };
    let right = RestoreKnowledge {
        version_id: Some(targets[1]),
        request_id: Uuid::new_v4(),
        expected_generation: 4,
    };
    let (a, b) = tokio::join!(
        store.restore_knowledge(user, project, &left),
        store.restore_knowledge(user, project, &right)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(
        if a.is_err() { a } else { b },
        Err(Error::Conflict)
    ));
    let pending = store
        .start(
            user,
            project,
            &StartMaintenance {
                request_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
    let first = store.claim("lease-test").await.unwrap().unwrap();
    assert_eq!(first.run.id, pending.id);
    store.renew(&first).await.unwrap();
    sqlx::query(
        "UPDATE maintenance_runs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(first.run.id)
    .execute(sql)
    .await
    .unwrap();
    let second = store.claim("lease-test").await.unwrap().unwrap();
    assert!(second.generation > first.generation);
    assert!(matches!(
        store.finish(&first, None, Some("STALE_EXECUTION")).await,
        Err(Error::Conflict)
    ));
    store
        .finish(&second, None, Some("ISOLATED_FENCE_CHECK"))
        .await
        .unwrap();
    assert_eq!(
        store
            .get(user, project, pending.id)
            .await
            .unwrap()
            .error_code
            .as_deref(),
        Some("ISOLATED_FENCE_CHECK")
    );
    for phase in ["summary", "readback"] {
        let pending = store
            .start(
                user,
                project,
                &StartMaintenance {
                    request_id: Uuid::new_v4(),
                },
            )
            .await
            .unwrap();
        let engine = MaintenanceEngine {
            store: store.clone(),
            sources: sources.clone(),
            model: Arc::new(InvalidPhaseModel {
                inner: Model { plan: plan.clone() },
                phase,
            }),
            budget: MaintenanceBudget::default(),
        };
        assert!(engine.tick().await.unwrap());
        let failed = store.get(user, project, pending.id).await.unwrap();
        assert!(matches!(failed.status, MaintenanceStatus::Incomplete));
        assert!(failed.candidate.is_none());
        assert!(matches!(
            store
                .publish_knowledge(
                    user,
                    project,
                    pending.id,
                    &PublishKnowledge {
                        request_id: Uuid::new_v4(),
                        expected_generation: 5
                    }
                )
                .await,
            Err(Error::InvalidInput { .. })
        ));
        if phase == "summary" {
            assert!(
                failed.model_calls <= failed.coverage.total_segments as u32 + 3,
                "summary must not repeat indefinitely"
            );
        }
        if phase == "readback" {
            let points = store
                .checkpoints(user, project, pending.id, 1, 100)
                .await
                .unwrap();
            assert!(!points.items.iter().any(|p| p.phase == "readback_complete"));
        }
    }
    let pending = store
        .start(
            user,
            project,
            &StartMaintenance {
                request_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
    let navigation = Arc::new(NavigationModel {
        inner: Model { plan: plan.clone() },
        requested: std::sync::atomic::AtomicBool::new(false),
        saw_original: std::sync::atomic::AtomicBool::new(false),
    });
    let engine = MaintenanceEngine {
        store: store.clone(),
        sources: sources.clone(),
        model: navigation.clone(),
        budget: MaintenanceBudget::default(),
    };
    assert!(engine.tick().await.unwrap());
    let ready = store.get(user, project, pending.id).await.unwrap();
    assert!(
        matches!(ready.status, MaintenanceStatus::Ready),
        "{:?}",
        ready.error_code
    );
    assert!(
        navigation
            .saw_original
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    let points = store
        .checkpoints(user, project, pending.id, 1, 100)
        .await
        .unwrap();
    assert!(points.items.iter().any(|p| p.phase == "index_read"));
    assert!(
        !points
            .items
            .iter()
            .any(|p| p.phase == "readback" && p.references.iter().any(|r| r.kind == "summary"))
    );
    let pending = store
        .start(
            user,
            project,
            &StartMaintenance {
                request_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
    let repair = Arc::new(RepairModel {
        inner: Model { plan: plan.clone() },
        bad_plan_sent: std::sync::atomic::AtomicBool::new(false),
        saw_feedback: std::sync::atomic::AtomicBool::new(false),
    });
    let engine = MaintenanceEngine {
        store: store.clone(),
        sources: sources.clone(),
        model: repair.clone(),
        budget: MaintenanceBudget::default(),
    };
    assert!(engine.tick().await.unwrap());
    assert!(matches!(
        store.get(user, project, pending.id).await.unwrap().status,
        MaintenanceStatus::Ready
    ));
    assert!(
        repair
            .saw_feedback
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    let pending = store
        .start(
            user,
            project,
            &StartMaintenance {
                request_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
    let confused = Arc::new(PhaseConfusionModel {
        inner: Model { plan },
        wrong_sent: std::sync::atomic::AtomicBool::new(false),
        saw_feedback: std::sync::atomic::AtomicBool::new(false),
    });
    let engine = MaintenanceEngine {
        store: store.clone(),
        sources: sources.clone(),
        model: confused.clone(),
        budget: MaintenanceBudget::default(),
    };
    assert!(engine.tick().await.unwrap());
    assert!(matches!(
        store.get(user, project, pending.id).await.unwrap().status,
        MaintenanceStatus::Ready
    ));
    assert!(
        confused
            .saw_feedback
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    // An impossible call budget is rejected before spending even one provider call.
    let pending = store
        .start(
            user,
            project,
            &StartMaintenance {
                request_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
    let engine = MaintenanceEngine {
        store: store.clone(),
        sources: sources.clone(),
        model: Arc::new(Model {
            plan: MaintenancePlan {
                strategy: MaintenanceStrategy::Keep,
                reason: "budget preflight".into(),
                expected_benefit: "no wasted calls".into(),
                actions: vec![],
            },
        }),
        budget: MaintenanceBudget {
            max_calls: 1,
            ..MaintenanceBudget::default()
        },
    };
    assert!(engine.tick().await.unwrap());
    let failed = store.get(user, project, pending.id).await.unwrap();
    assert_eq!(failed.model_calls, 0);
    assert!(failed.candidate.is_none());
    assert_eq!(
        failed.error_code.as_deref(),
        Some("MODEL_BUDGET_INSUFFICIENT_FOR_REVIEW")
    );
}

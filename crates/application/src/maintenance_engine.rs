mod context;
mod execution;
mod readback;
use crate::{MaintenanceLease, MaintenanceSources, MaintenanceStore};
use context::*;
use nexofolio_contracts::*;
use nexofolio_rebuild::{
    MaintenanceModel, ReviewSegment, materialize_maintenance, required_reads, review_segments,
    validate_segment,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
#[derive(Clone, Debug, serde::Serialize)]
pub struct MaintenanceBudget {
    pub context_tokens: usize,
    pub max_calls: u32,
    pub retries: u32,
}
impl Default for MaintenanceBudget {
    fn default() -> Self {
        Self {
            context_tokens: 65536,
            max_calls: 256,
            retries: 2,
        }
    }
}
pub struct MaintenanceEngine {
    pub store: Arc<dyn MaintenanceStore>,
    pub model: Arc<dyn MaintenanceModel>,
    pub sources: Arc<dyn MaintenanceSources>,
    pub budget: MaintenanceBudget,
}
fn invalid(s: &str) -> Error {
    Error::InvalidInput { message: s.into() }
}
fn encoded(v: &impl serde::Serialize) -> Value {
    serde_json::to_value(v).expect("serializes")
}
fn bytes(v: &Value) -> usize {
    serde_json::to_vec(v).expect("serializes").len()
}
impl MaintenanceEngine {
    pub async fn tick(&self) -> Result<bool> {
        let settings =
            json!({"model":self.model.identity(),"budget":self.budget,"engine":"full-review-6-batched"})
                .to_string();
        let Some(lease) = self.store.claim(&settings).await? else {
            return Ok(false);
        };
        match self.execute(&lease).await {
            Ok(c) => {
                self.store
                    .finish(
                        &lease,
                        Some(&c),
                        if c.issues.is_empty() && c.review.structurally_valid {
                            None
                        } else {
                            Some("CANDIDATE_VALIDATION_FAILED")
                        },
                    )
                    .await?
            }
            Err(e) => {
                let code = match &e {
                    Error::Conflict => "EXECUTION_OWNERSHIP_LOST",
                    Error::NotConfigured { .. } => "CAPABILITY_NOT_CONFIGURED",
                    Error::InvalidInput { message }
                        if message.chars().all(|c| c.is_ascii_uppercase() || c == '_') =>
                    {
                        message.as_str()
                    }
                    _ => "MAINTENANCE_EXECUTION_FAILED",
                };
                if !matches!(e, Error::Conflict) {
                    self.store.finish(&lease, None, Some(code)).await?;
                }
            }
        }
        Ok(true)
    }
    fn input_limit(&self) -> usize {
        self.budget.context_tokens * 60 / 100
    }
    fn data_limit(&self) -> usize {
        (self.input_limit().saturating_sub(12000) / 4).max(2048)
    }
    async fn ask(
        &self,
        lease: &MaintenanceLease,
        phase: &str,
        input: Value,
    ) -> Result<MaintenanceReply> {
        let mut feedback = Value::Null;
        for _ in 0..=self.budget.retries {
            self.store.renew(lease).await?;
            let mut wrapped = json!({"input":input,"previous_error":feedback});
            if let Some(image) = wrapped["input"]
                .as_object_mut()
                .and_then(|o| o.remove("image_data_url"))
            {
                wrapped["image_data_url"] = image;
            }
            let request = self.model.prepare(
                phase,
                wrapped,
                (self.budget.context_tokens * 40 / 100).min(16384),
            )?;
            // UTF-8 byte count is a conservative text-token upper bound. Never silently truncate.
            let mut text_request = request.clone();
            let mut vision_budget = 0;
            if let Some(messages) = text_request["messages"].as_array_mut() {
                for message in messages {
                    if let Some(parts) = message["content"].as_array_mut() {
                        for part in parts {
                            if part["type"] == "image_url" {
                                part["image_url"] = Value::Null;
                                vision_budget += 8192;
                            }
                        }
                    }
                }
            }
            if bytes(&text_request) + vision_budget > self.input_limit() {
                return Err(invalid("MODEL_INPUT_BUDGET_EXHAUSTED"));
            }
            let call = self
                .store
                .begin_call(lease, &request, self.budget.max_calls)
                .await?;
            match self.model.invoke_tracked(&request).await {
                Ok(invocation) => {
                    self.store
                        .end_call(
                            lease,
                            call,
                            &json!({"output":invocation.content,"usage":invocation.usage}),
                        )
                        .await?;
                    let raw = invocation.content;
                    match serde_json::from_value::<MaintenanceReply>(raw) {
                        Ok(reply)
                            if valid_reply(
                                phase,
                                &reply,
                                &input,
                                &lease.snapshot,
                                self.data_limit() / 4,
                            ) =>
                        {
                            return Ok(reply);
                        }
                        Ok(_) => feedback = phase_feedback(phase, self.data_limit() / 4),
                        Err(_) => feedback = phase_feedback(phase, self.data_limit() / 4),
                    }
                }
                Err(error) => {
                    let code = match &error {
                        Error::InvalidInput { message } => message.as_str(),
                        Error::Unavailable { component } => component,
                        Error::NotConfigured { capability } => capability,
                        _ => "PROVIDER_CALL_FAILED",
                    };
                    self.store
                        .end_call(lease, call, &json!({"error":code}))
                        .await?;
                    feedback = json!(
                        "Previous call failed or was incomplete; retry with complete output."
                    );
                }
            }
        }
        Err(invalid("MODEL_RETRY_LIMIT_REACHED"))
    }
    async fn review(
        &self,
        lease: &MaintenanceLease,
        segment: &ReviewSegment,
    ) -> Result<SegmentReview> {
        let result=self.ask(lease,"review",json!({"snapshot_id":lease.snapshot.id,"segment":segment,"requirement":"Review semantic knowledge for every unit exactly once, not whether JSON types should change. A correct field type can still lack a useful description, captured parameter relation or observed enum mapping. Use evidence_summary representative.reference pointers to identify justified maintenance or needed readback; never infer a closed enum from samples. Keep protocol boilerplate and unconstrained IDs from becoming spurious enums. schema:null means no observed structure, not a JSON-null sample. Check capture state. Existing model conclusions are not independent evidence."})).await?;
        match result {
            MaintenanceReply::Review { review } => Ok(review),
            _ => Err(invalid("REVIEW_COVERAGE_INCOMPLETE")),
        }
    }
    async fn summarize(
        &self,
        lease: &MaintenanceLease,
        id: &str,
        input: Value,
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<MaintenanceCheckpoint> {
        if let Some(c) = saved.get(id) {
            return Ok(c.clone());
        }
        let MaintenanceReply::Summary { summary } =
            self.ask(lease, "summary", input.clone()).await?
        else {
            return Err(invalid("SUMMARY_CONTRACT_INVALID"));
        };
        let c = MaintenanceCheckpoint {
            id: id.into(),
            phase: "summary".into(),
            references: vec![],
            review: None,
            summary: Some(summary),
            data: input,
        };
        self.store.checkpoint(lease, &c, coverage).await?;
        saved.insert(id.into(), c.clone());
        Ok(c)
    }
    async fn global(
        &self,
        lease: &MaintenanceLease,
        mut items: Vec<Value>,
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<Value> {
        let mut expanded = vec![];
        for item in items {
            if bytes(&item) > self.data_limit() {
                for (part, entries) in fragment(&item, self.data_limit() / 2)?
                    .into_iter()
                    .enumerate()
                {
                    expanded.push(json!({"index_fragment":true,"part":part,"entries":entries}));
                }
            } else {
                expanded.push(item)
            }
        }
        items = expanded;
        for level in 0..16 {
            let before_bytes = bytes(&json!(items));
            if bytes(&json!(items)) <= self.data_limit() {
                return Ok(json!(items));
            }
            let groups = groups(items, self.data_limit())?;
            let mut next = vec![];
            for (index, group) in groups.into_iter().enumerate() {
                let key = format!("summary:{level}:{index}:{}", digest(&encoded(&group)));
                let c=self.summarize(lease,&key,json!({"level":level,"items":group,"summary_max_utf8_bytes":self.data_limit()/4,"instruction":"Keep only useful findings, conflicts, uncertain points and links within the byte budget. Do not repeat ordinary field types. Full input items and IDs are preserved in the machine index and this checkpoint. Summaries are navigation, never original evidence. Never abbreviate any ID you do include."}),coverage,saved).await?;
                next.push(json!({"summary_id":key,"summary":c.summary}));
            }
            if bytes(&json!(next)) >= before_bytes {
                return Err(invalid("SUMMARY_NOT_CONVERGING"));
            }
            items = next;
        }
        Err(invalid("GLOBAL_INDEX_BUDGET_EXHAUSTED"))
    }
}

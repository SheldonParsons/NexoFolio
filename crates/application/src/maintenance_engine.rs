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
            json!({"model":self.model.identity(),"budget":self.budget,"engine":"full-review-11-progressing-reads","structure_rule":nexofolio_contracts::STRUCTURE_ALGORITHM})
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
        let result = self
            .ask(lease, "review", review_input(lease.snapshot.id, segment))
            .await?;
        match result {
            MaintenanceReply::Review { review } => Ok(review),
            _ => Err(invalid("REVIEW_COVERAGE_INCOMPLETE")),
        }
    }
    async fn review_checkpoint(
        &self,
        lease: &MaintenanceLease,
        segment: &ReviewSegment,
        existing: Option<MaintenanceCheckpoint>,
    ) -> Result<MaintenanceCheckpoint> {
        if let Some(checkpoint) = existing {
            validate_segment(
                segment,
                checkpoint
                    .review
                    .as_ref()
                    .ok_or_else(|| invalid("CHECKPOINT_INVALID"))?,
            )?;
            return Ok(checkpoint);
        }
        let review = self.review(lease, segment).await?;
        Ok(MaintenanceCheckpoint {
            id: segment.id.clone(),
            phase: "review".into(),
            references: segment
                .units
                .iter()
                .map(|u| KnowledgeEvidenceRef {
                    kind: "field".into(),
                    id: u.field_id.clone(),
                })
                .collect(),
            review: Some(review),
            summary: None,
            data: json!({"units":segment.units.iter().map(|u|&u.id).collect::<Vec<_>>()}),
        })
    }
    async fn save_navigation(
        &self,
        lease: &MaintenanceLease,
        id: &str,
        source: Value,
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<()> {
        for checkpoint in navigation_pages(id, source, self.data_limit())? {
            self.store.checkpoint(lease, &checkpoint, coverage).await?;
            saved.insert(checkpoint.id.clone(), checkpoint);
        }
        Ok(())
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
        items = summary_items(items, self.data_limit())?;
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

/// Deterministic preflight: real serialized requests, plus bounded downstream work.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MaintenanceWorkEstimate {
    pub review_calls: usize,
    pub summary_calls_upper_bound: usize,
    pub planning_and_readback_reserve: usize,
    pub retry_reserve: usize,
    pub maximum_review_request_bytes: usize,
    pub input_limit_bytes: usize,
}
impl MaintenanceWorkEstimate {
    pub fn reserved_calls(&self) -> usize {
        self.summary_calls_upper_bound + self.planning_and_readback_reserve + self.retry_reserve
    }
    pub fn total_calls(&self) -> usize {
        self.review_calls + self.reserved_calls()
    }
}
pub fn review_input(snapshot_id: uuid::Uuid, segment: &ReviewSegment) -> Value {
    json!({"snapshot_id":snapshot_id,"segment":segment.model_input(),"summary_max_json_bytes":nexofolio_rebuild::REVIEW_SUMMARY_BYTES,"requirement":"Review every unit exactly once. unit.context_ref indexes segment.contexts: its reference_ref indexes segment.references; combine that reference base with unit.path, restoring observation.path when absent. Combine shared context with unit.context, likewise observation_source.path defaults to unit.path. Ancestor definitions use reference_ref plus path in the same way. unit.ancestors resolve through ancestor_definitions. All original structure and evidence navigation are present. Summary: only useful findings/conflicts/readback needs, within summary_max_json_bytes (serialized JSON string); keep ordinary types and field-name translations out. Correct JSON types alone do not establish semantic quality. Return keep for mere restatements; propose change only for useful supported knowledge. Distinguish schema_not_observed=true from schema type null with an observed null sample. Few values are not a closed enum, ID values are not enums. Old model conclusions are not evidence. Existing observed values are already available as examples; do not propose enums or descriptions merely to repeat them. Protocol headers/credentials and unconstrained IDs are not business enums from passive samples. For every parameter_link_candidate or parameter_link_counterexample representative, cite its fact reference in that unit assessment even when choosing keep; a field reference alone does not acknowledge relation evidence. Explain ambiguity, search_complete=false and necessary source readback. Unresolved sources should be needs_evidence, not dismissed because the JSON type matches. Reflect unresolved relationships in the summary. Do not invent labels or closed constraints."})
}
pub fn prepare_maintenance_review(
    snapshot: &KnowledgeSnapshot,
    model: &dyn nexofolio_rebuild::MaintenanceModel,
    budget: &MaintenanceBudget,
) -> Result<(Vec<ReviewSegment>, MaintenanceWorkEstimate)> {
    if budget.context_tokens < 24000 || budget.max_calls == 0 {
        return Err(invalid("INVALID_MODEL_BUDGET"));
    }
    let input_limit = budget.context_tokens * 60 / 100;
    let output = (budget.context_tokens * 40 / 100).min(16384);
    let mut limit = input_limit.saturating_sub(6000);
    // JSON escaping and provider-specific wrappers are measured, not guessed as tokens.
    for _ in 0..4 {
        let segments = review_segments(snapshot, limit)?;
        let maximum = segments
            .iter()
            .map(|s| {
                model
                    .prepare(
                        "review",
                        json!({"input":review_input(snapshot.id,s),"previous_error":null}),
                        output,
                    )
                    .map(|r| bytes(&r))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .unwrap_or(0);
        if maximum + 1024 > input_limit {
            limit = limit.saturating_sub(maximum + 1024 - input_limit);
            continue;
        }
        let data_limit = (input_limit.saturating_sub(12000) / 4).max(2048);
        let mut items = global_navigation(snapshot);
        for s in &segments {
            let mut navigation = review_navigation(&SegmentReview {
                segment_id: s.id.clone(),
                summary: "x".repeat(nexofolio_rebuild::REVIEW_SUMMARY_BYTES),
                assessments: vec![],
            });
            navigation["dispositions"] =
                json!({"keep":9999,"change":9999,"conflict":9999,"needs_evidence":9999});
            items.push(navigation);
        }
        let summary_calls = summary_call_bound(items, data_limit)?;
        let review_calls = segments.len();
        return Ok((
            segments,
            MaintenanceWorkEstimate {
                review_calls,
                summary_calls_upper_bound: summary_calls,
                planning_and_readback_reserve: 16,
                retry_reserve: budget.retries as usize * 4,
                maximum_review_request_bytes: maximum,
                input_limit_bytes: input_limit,
            },
        ));
    }
    Err(invalid("MODEL_INPUT_BUDGET_EXHAUSTED"))
}
fn summary_items(items: Vec<Value>, limit: usize) -> Result<Vec<Value>> {
    let mut expanded = vec![];
    for item in items {
        if bytes(&item) > limit {
            for (part, entries) in fragment(&item, limit / 2)?.into_iter().enumerate() {
                expanded.push(json!({"index_fragment":true,"part":part,"entries":entries}));
            }
        } else {
            expanded.push(item);
        }
    }
    Ok(expanded)
}
fn summary_call_bound(mut items: Vec<Value>, limit: usize) -> Result<usize> {
    items = summary_items(items, limit)?;
    let mut calls = 0;
    for _ in 0..16 {
        if bytes(&json!(items)) <= limit {
            return Ok(calls);
        }
        let count = groups(items, limit)?.len();
        calls += count;
        // Serialized summaries can escape every byte (e.g. control characters).
        // Oversized/unhelpful replies are rejected at the shared summary validator.
        items = (0..count)
            .map(|_| json!({"summary_id":"x".repeat(100),"summary":"x".repeat(limit / 4 - 2)}))
            .collect();
    }
    Err(invalid("GLOBAL_INDEX_BUDGET_EXHAUSTED"))
}

pub fn planning_window(
    model: &dyn nexofolio_rebuild::MaintenanceModel,
    budget: &MaintenanceBudget,
    mut input: Value,
    pending: &[Value],
    sources: &std::collections::BTreeMap<String, Value>,
) -> Result<(Value, usize, Vec<String>)> {
    let fits = |input: &Value| -> Result<bool> {
        Ok(bytes(&model.prepare(
            "plan",
            json!({"input":input,"previous_error":null}),
            (budget.context_tokens * 40 / 100).min(16384),
        )?) + 1024
            <= (budget.context_tokens * 60 / 100))
    };
    if !fits(&input)? {
        return Err(invalid("PLANNING_WORKSET_EXCEEDS_BUDGET"));
    }
    let mut count = 0;
    let mut delivered_keys = vec![];
    // Guarantee progress for BOTH queues before filling spare space. Failure asks
    // the caller to compact repeated background context, never to starve originals.
    if let Some(first) = pending.first() {
        input["pending_originals"]
            .as_array_mut()
            .unwrap()
            .push(first.clone());
        count = 1;
    }
    if let Some((key, value)) = sources.first_key_value() {
        input["exact_read_sources"][key] = value.clone();
        delivered_keys.push(key.clone());
    }
    if !fits(&input)? {
        return Err(invalid("PLANNING_WORKSET_EXCEEDS_BUDGET"));
    }
    for item in pending.iter().skip(count) {
        input["pending_originals"]
            .as_array_mut()
            .unwrap()
            .push(item.clone());
        if !fits(&input)? {
            input["pending_originals"].as_array_mut().unwrap().pop();
            break;
        }
        count += 1;
    }
    for (key, value) in sources.iter().skip(delivered_keys.len()) {
        input["exact_read_sources"][key] = value.clone();
        if fits(&input)? {
            delivered_keys.push(key.clone());
        } else {
            input["exact_read_sources"]
                .as_object_mut()
                .unwrap()
                .remove(key);
        }
    }
    input["queued_originals"] = json!(pending.len() - count);
    input["queued_navigation"] = json!(sources.len() - delivered_keys.len());
    Ok((input, count, delivered_keys))
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn summary_bound_includes_higher_levels_and_json_escaping() {
        let limit = 2048;
        let items: Vec<_> = (0..100)
            .map(|n| json!({"id":n,"finding":"x".repeat(600)}))
            .collect();
        let bound = summary_call_bound(items.clone(), limit).unwrap();
        let mut current = items;
        let mut calls = 0;
        for _ in 0..16 {
            if bytes(&json!(current)) <= limit {
                break;
            }
            let chunks = groups(current, limit).unwrap();
            calls += chunks.len();
            current = chunks
                .iter()
                .map(|_| json!({"summary_id":"x".repeat(90),"summary":"x".repeat(limit/4-2)}))
                .collect();
        }
        assert!(calls > 1 && calls <= bound);
        assert!(bytes(&json!(current)) <= limit);
        assert!(bytes(&json!("\u{0001}".repeat(100))) > limit / 4);
    }
    struct WindowModel;
    #[async_trait::async_trait]
    impl nexofolio_rebuild::MaintenanceModel for WindowModel {
        fn identity(&self) -> Value {
            json!({"fixture":"window"})
        }
        fn prepare(&self, _: &str, input: Value, _: usize) -> Result<Value> {
            Ok(input)
        }
        async fn invoke(&self, _: &Value) -> Result<Value> {
            unreachable!("offline test")
        }
    }
    #[test]
    fn planning_window_delivers_every_queued_item_without_oversized_requests() {
        let budget = MaintenanceBudget {
            context_tokens: 24000,
            ..Default::default()
        };
        let base =
            json!({"padding":"x".repeat(9000),"exact_read_sources":{},"pending_originals":[]});
        let mut sources: std::collections::BTreeMap<_, _> = (0..16)
            .map(|n| {
                (
                    format!("navigation:{n}"),
                    json!({"id":n,"text":"汉字".repeat(200)}),
                )
            })
            .collect();
        let mut originals:Vec<_>=(0..39).map(|n|json!({"reference":{"kind":"interface","id":n},"originals":[{"content":"x".repeat(1800)}]})).collect();
        let mut delivered = vec![];
        let mut nav = HashSet::new();
        let mut rounds = 0;
        while !sources.is_empty() || !originals.is_empty() {
            let (input, n, keys) =
                planning_window(&WindowModel, &budget, base.clone(), &originals, &sources).unwrap();
            assert!(
                bytes(&json!({"input":input,"previous_error":null})) + 1024
                    <= budget.context_tokens * 60 / 100
            );
            assert!(
                originals.is_empty() || n > 0,
                "navigation must not starve queued originals"
            );
            assert_eq!(input["pending_originals"], json!(&originals[..n]));
            delivered.extend(originals.drain(..n));
            for key in keys {
                assert_eq!(
                    input["exact_read_sources"][&key],
                    sources.remove(&key).unwrap()
                );
                assert!(nav.insert(key));
            }
            rounds += 1;
            assert!(rounds <= 55);
        }
        assert_eq!(delivered.len(), 39);
        assert_eq!(nav.len(), 16);
        assert!(rounds > 1);
    }
    #[test]
    fn oversized_background_fails_instead_of_sending_only_navigation() {
        let budget = MaintenanceBudget {
            context_tokens: 24000,
            ..Default::default()
        };
        let base =
            json!({"padding":"x".repeat(12000),"exact_read_sources":{},"pending_originals":[]});
        let originals = vec![json!({"original":"x".repeat(1800)})];
        let sources = std::collections::BTreeMap::from([("index".into(), json!({"index":true}))]);
        assert!(
            planning_window(&WindowModel, &budget, base.clone(), &originals, &sources).is_err()
        );
        let mut compact = base;
        compact["padding"] = json!({"read_ref":"saved-background"});
        let (_, n, keys) =
            planning_window(&WindowModel, &budget, compact, &originals, &sources).unwrap();
        assert_eq!(n, 1);
        assert_eq!(keys, vec!["index"]);
    }
    #[test]
    fn repeated_navigation_cannot_starve_originals_or_erase_evidence_previews() {
        let budget = MaintenanceBudget {
            context_tokens: 24000,
            ..Default::default()
        };
        let base =
            json!({"padding":"x".repeat(9000),"exact_read_sources":{},"pending_originals":[]});
        let sources = std::collections::BTreeMap::from([(
            "repeated-index".into(),
            json!({"text":"x".repeat(1400)}),
        )]);
        let mut originals: Vec<_> = (0..39)
            .map(|id| json!({"id":id,"content":"x".repeat(1800)}))
            .collect();
        let mut delivered = 0;
        while !originals.is_empty() {
            let (_, n, keys) =
                planning_window(&WindowModel, &budget, base.clone(), &originals, &sources).unwrap();
            assert!(n > 0);
            assert_eq!(keys.len(), 1);
            delivered += n;
            originals.drain(..n);
        }
        assert_eq!(delivered, 39);
        let r = KnowledgeEvidenceRef {
            kind: "fact".into(),
            id: "known".into(),
        };
        let hints = json!({"items":[{"evidence":{"representatives":[{"reference":r,"summary":{"value":23}}]}}]});
        let out = hint_context(&hints, &HashSet::from([r]));
        assert_eq!(
            out["items"][0]["evidence"]["representatives"][0]["summary"]["value"],
            23
        );
        assert_eq!(
            out["items"][0]["evidence"]["representatives"][0]["already_read"],
            true
        );
    }
    #[test]
    fn long_string_pages_round_trip_unicode_and_escaping() {
        let text = "汉字\"\n🧪".repeat(3000);
        let parts = fragment(&json!({"body":text}), 2048).unwrap();
        assert!(parts.len() > 1);
        let mut restored = String::new();
        for page in &parts {
            assert!(bytes(&json!(page)) <= 2048);
            for item in page {
                assert_eq!(item["pointer"], "/body");
                assert_eq!(item["utf8_start"], json!(restored.len()));
                restored.push_str(item["value"].as_str().unwrap());
            }
        }
        assert_eq!(restored, text);
        let pages = navigation_pages("test", json!({"body":text}), 2048).unwrap();
        assert!(pages.iter().all(|p| bytes(&p.data) <= 2048));
    }
}

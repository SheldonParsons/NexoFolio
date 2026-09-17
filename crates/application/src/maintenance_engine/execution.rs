use super::*;

impl MaintenanceEngine {
    pub(super) async fn execute(&self, lease: &MaintenanceLease) -> Result<MaintenanceCandidate> {
        if self.budget.context_tokens < 24000 || self.budget.max_calls == 0 {
            return Err(invalid("INVALID_MODEL_BUDGET"));
        }
        let snapshot = &lease.snapshot;
        snapshot
            .inputs
            .as_ref()
            .ok_or_else(|| invalid("SNAPSHOT_INPUTS_UNAVAILABLE_RECREATE"))?;
        let (segments, estimate) =
            prepare_maintenance_review(snapshot, self.model.as_ref(), &self.budget)?;
        let mut saved: HashMap<_, _> = self
            .store
            .saved_checkpoints(lease)
            .await?
            .into_iter()
            .map(|c| (c.id.clone(), c))
            .collect();
        // Fail before paying for a review that cannot possibly leave room to plan.
        let unread = segments
            .iter()
            .filter(|segment| !saved.contains_key(&segment.id))
            .count();
        let used = self.store.call_count(lease).await?;
        let mut estimate = estimate;
        if unread == 0
            && saved
                .get("coverage")
                .is_some_and(|c| c.data["complete"] == true)
        {
            estimate.summary_calls_upper_bound = 0;
        }
        let mut coverage = ReviewCoverage {
            total_interfaces: snapshot.interfaces.len(),
            total_fields: snapshot.fields.len(),
            reviewed_fields: 0,
            total_segments: segments.len(),
            completed_segments: 0,
            complete: false,
        };
        self.store
            .checkpoint(
                lease,
                &MaintenanceCheckpoint {
                    id: "execution-budget".into(),
                    phase: "preflight".into(),
                    references: vec![],
                    review: None,
                    summary: None,
                    data: encoded(&estimate),
                },
                &coverage,
            )
            .await?;
        if unread.saturating_add(estimate.reserved_calls())
            > self.budget.max_calls.saturating_sub(used) as usize
        {
            return Err(invalid("MODEL_BUDGET_INSUFFICIENT_FOR_REVIEW"));
        }
        let index_checkpoint = MaintenanceCheckpoint {
            id: "snapshot-index".into(),
            phase: "review".into(),
            references: snapshot
                .fields
                .iter()
                .map(|f| KnowledgeEvidenceRef {
                    kind: "field".into(),
                    id: f.id.clone(),
                })
                .collect(),
            review: None,
            summary: None,
            data: compact_index(snapshot),
        };
        for c in paginate_index(index_checkpoint, self.data_limit(), self.input_limit() / 2)? {
            self.store.checkpoint(lease, &c, &coverage).await?;
            saved.insert(c.id.clone(), c);
        }
        let raw_hints = nexofolio_rebuild::field_maintenance_hints(snapshot);
        let field_hints = if bytes(&json!(raw_hints)) <= self.data_limit() {
            json!({"items":raw_hints})
        } else {
            let total = raw_hints.len();
            let mut refs = vec![];
            for (n, items) in groups(raw_hints, self.data_limit().saturating_sub(256))?
                .into_iter()
                .enumerate()
            {
                let id = format!("semantic-hints:{n}");
                let c = MaintenanceCheckpoint {
                    id: id.clone(),
                    phase: "index".into(),
                    references: vec![],
                    review: None,
                    summary: None,
                    data: json!({"items":items}),
                };
                self.store.checkpoint(lease, &c, &coverage).await?;
                saved.insert(id.clone(), c);
                refs.push(json!({"kind":"summary","id":id}));
            }
            json!({"total_fields":total,"pages":refs,"complete_index":true})
        };
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut required: HashMap<String, usize> = HashMap::new();
        for s in &segments {
            for u in &s.units {
                *required.entry(u.field_id.clone()).or_default() += 1;
            }
        }
        let mut global = global_navigation(snapshot);
        // Independent reads only; checkpoint each success immediately. Drain the
        // bounded batch on failure so completed peers are not discarded or retried.
        for batch in segments.chunks(4) {
            let remaining = segments
                .iter()
                .filter(|s| !saved.contains_key(&s.id))
                .count();
            let used = self.store.call_count(lease).await? as usize;
            if remaining
                + estimate.summary_calls_upper_bound
                + estimate.planning_and_readback_reserve
                > (self.budget.max_calls as usize).saturating_sub(used)
            {
                return Err(invalid("MODEL_BUDGET_INSUFFICIENT_FOR_REVIEW"));
            }
            let batch_coverage = &coverage;
            let results = futures_util::future::join_all(batch.iter().map(|s| {
                let existing = saved.get(&s.id).cloned();
                async move {
                    let already_saved = existing.is_some();
                    let checkpoint = self.review_checkpoint(lease, s, existing).await?;
                    if !already_saved {
                        // Persist within the polled future: an outer write must not wait
                        // on a row lock held by a sibling future that is no longer polled.
                        self.store
                            .checkpoint(lease, &checkpoint, batch_coverage)
                            .await?;
                    }
                    Ok::<_, Error>(checkpoint)
                }
            }))
            .await;
            let mut failure = None;
            for (s, result) in batch.iter().zip(results) {
                let c = match result {
                    Ok(c) => c,
                    Err(error) => {
                        failure.get_or_insert(error);
                        continue;
                    }
                };
                for u in &s.units {
                    *counts.entry(u.field_id.clone()).or_default() += 1;
                }
                coverage.completed_segments += 1;
                coverage.reviewed_fields = counts
                    .iter()
                    .filter(|(k, n)| required.get(*k) == Some(*n))
                    .count();
                saved.insert(c.id.clone(), c);
            }
            self.store
                .checkpoint(
                    lease,
                    &MaintenanceCheckpoint {
                        id: "review-progress".into(),
                        phase: "review".into(),
                        references: vec![],
                        review: None,
                        summary: None,
                        data: Value::Null,
                    },
                    &coverage,
                )
                .await?;
            if let Some(error) = failure {
                return Err(error);
            }
        }
        if coverage.reviewed_fields != snapshot.fields.len()
            || snapshot.interfaces.iter().any(|i| {
                !snapshot
                    .fields
                    .iter()
                    .any(|f| f.reference.interface_id == i.interface_id)
            })
        {
            return Err(invalid("REVIEW_COVERAGE_INCOMPLETE"));
        }
        coverage.complete = true;
        // Planning input order remains deterministic regardless of completion order.
        for segment in &segments {
            global.push(review_navigation(
                saved[&segment.id]
                    .review
                    .as_ref()
                    .expect("validated review"),
            ));
        }
        let index = self.global(lease, global, &coverage, &mut saved).await?;
        let c = MaintenanceCheckpoint {
            id: "coverage".into(),
            phase: "planning".into(),
            references: snapshot
                .fields
                .iter()
                .map(|f| KnowledgeEvidenceRef {
                    kind: "field".into(),
                    id: f.id.clone(),
                })
                .collect(),
            review: None,
            summary: None,
            data: encoded(&coverage),
        };
        self.store.checkpoint(lease, &c, &coverage).await?;
        saved.insert(c.id.clone(), c);
        let mut read_refs: HashSet<KnowledgeEvidenceRef> = saved
            .values()
            .filter(|c| c.phase == "readback_complete")
            .flat_map(|c| c.references.clone())
            .collect();
        expand_interface_reads(snapshot, &mut read_refs);
        let mut exact_sources: std::collections::BTreeMap<String, Value> = saved
            .values()
            .filter(|c| c.phase == "index_read")
            .map(|c| (c.id.clone(), c.data["source"].clone()))
            .collect();
        self.save_navigation(
            lease,
            "field-hints",
            field_hints.clone(),
            &coverage,
            &mut saved,
        )
        .await?;
        self.save_navigation(lease, "global-index", index.clone(), &coverage, &mut saved)
            .await?;
        let mut last_plan = Value::Null;
        let mut validation_feedback = Value::Null;
        let mut validation_failures = 0;
        let mut pending_originals = Vec::new();
        for _ in 0..self.budget.max_calls {
            let mut reports: Vec<Value> = saved
                .values()
                .filter(|c| c.phase == "readback_complete")
                .map(|c| json!({"reference":c.references,"readback_id":c.id,"summary":c.summary}))
                .collect();
            reports.sort_by_key(Value::to_string);
            self.save_navigation(
                lease,
                "completed-read-index",
                json!({"items":reports}),
                &coverage,
                &mut saved,
            )
            .await?;
            let mut reports_index = json!({"completed_reads":reports.len(),"index":{"kind":"summary","id":"completed-read-index"},"meaning":"Completed read proofs and findings. Request the original resource kind/id to see its content again; summaries are navigation, not evidence."});
            let references: Vec<_> = reports
                .iter()
                .flat_map(|r| r["reference"].as_array().into_iter().flatten().cloned())
                .collect();
            if bytes(&json!(references)) <= self.data_limit() {
                reports_index["completed_references"] = json!(references);
            }
            if bytes(&last_plan) > self.data_limit() {
                self.save_navigation(
                    lease,
                    "previous-plan",
                    last_plan.clone(),
                    &coverage,
                    &mut saved,
                )
                .await?;
            }
            let previous_plan = last_plan.clone();
            let compact_hints = hint_context(&field_hints, &read_refs);
            let used = self.store.call_count(lease).await?;
            let catalog_index = if bytes(&encoded(&snapshot.catalog)) < self.data_limit() {
                encoded(&snapshot.catalog)
            } else {
                json!({"read_index":"snapshot-index"})
            };
            let interface_index = encoded(
                &snapshot
                    .interfaces
                    .iter()
                    .map(|i| json!({"interface_id":i.interface_id,"method":i.method,"path":i.path}))
                    .collect::<Vec<_>>(),
            );
            let interface_index = if bytes(&interface_index) < self.data_limit() {
                interface_index
            } else {
                json!({"read_index":"snapshot-index"})
            };
            let mut plan_input = json!({"snapshot_id":snapshot.id,"coverage":coverage,"full_index_ref":{"kind":"summary","id":"snapshot-index"},"coverage_manifest_ref":{"kind":"summary","id":"coverage"},"current_catalog":catalog_index,"all_interface_index":interface_index,"capabilities":{"vision":self.model.identity()["vision_enabled"]==true},"remaining_model_calls":self.budget.max_calls.saturating_sub(used),"global_index":index,"readback":reports_index,"exact_read_sources":{},"pending_originals":[],"queued_originals":pending_originals.len(),"queued_navigation":exact_sources.len(),"previous_plan":previous_plan,"validation_feedback":validation_feedback,"field_knowledge_hints":compact_hints,"maintenance_objectives":["Evaluate directory organization","Evaluate captured parameter-source relationships; uncertain links can remain inferred or needs_review","Evaluate observed values and UI label mappings as incomplete enum knowledge","Evaluate useful field descriptions; explain why each class changes or stays unchanged"],"read_kinds":["interface","field","fact","directory","image","summary","annotation","observation","source"],"instructions":"Coverage describes the declared review manifest, not complete capture or resolved adjudication. Read input_gaps. Observation-basis fields are read-only: never invent a revision ID, adopt a schema or write a formal annotation to them. legacy/needs_reassessment facts are visible history, not independent validated evidence. Observation materials preserve stored assessments; a reconstructed definition is explicitly separate. Frozen sources preserve captured_at and coverage. Full machine index and every reviewed unit are available by stable reference. Summaries are navigation; exact_read_sources contains original snapshot fields and facts. Read every target and support before changing it. pending_originals contains complete provided objects for this call; sample descriptors with contents_provided=false are verified links, not captured bodies. Request source IDs to inspect captured bodies when necessary. Consult completed_references and already_read evidence previews before requesting another read; do not reread solely to satisfy validation. Preserve useful changes in previous_plan while inspecting remaining originals. Completed originals remain readable by original kind/id; only a bounded working set is delivered now. Pending queued materials have not yet been delivered; return read with empty requests to continue delivery if needed. Small reads are merged into planning. When original_delivery=true, inspect the delivered originals and preserve or repair the prior draft; background is accessible by read_ref and will be restored for the final global decision. Use read with empty requests to advance remaining queued originals if no further specific material is needed. Prioritize useful changes that fit the remaining budget. Do not mechanically expand field names into low-value prose. Do not cite old annotations or summaries as independent evidence. Leave unavailable images unread. Never abbreviate IDs. field_knowledge_hints contains exact current FieldRefs and captured evidence pointers; do not claim these identities or evidence are unavailable without reading them. Prioritize justified business enum mappings and parameter-source clues: record observed values as incomplete and relationships as inferred/needs_review, not proven causes. Do not create enums from arbitrary IDs."});
            let delivering_originals = !pending_originals.is_empty();
            if delivering_originals {
                // Background is retained losslessly in checkpoints and restored for
                // the final global decision; it must not consume each delivery call.
                plan_input["current_catalog"] = json!({"read_index":"snapshot-index"});
                plan_input["all_interface_index"] = json!({"read_index":"snapshot-index"});
                plan_input["global_index"] =
                    json!({"read_ref":{"kind":"summary","id":"global-index"}});
                plan_input["field_knowledge_hints"] =
                    json!({"read_ref":{"kind":"summary","id":"field-hints"}});
                if bytes(&last_plan) > self.data_limit() {
                    plan_input["previous_plan"] = json!({"read_ref":{"kind":"summary","id":"previous-plan"},"strategy":last_plan["strategy"],"reason":last_plan["reason"]});
                }
            }
            plan_input["original_delivery"] = json!(delivering_originals);
            let mut window = planning_window(
                self.model.as_ref(),
                &self.budget,
                plan_input.clone(),
                &pending_originals,
                &exact_sources,
            );
            if window.is_err() {
                plan_input["current_catalog"] = json!({"read_index":"snapshot-index"});
                plan_input["global_index"] =
                    json!({"read_ref":{"kind":"summary","id":"global-index"}});
                plan_input["field_knowledge_hints"] =
                    json!({"read_ref":{"kind":"summary","id":"field-hints"}});
                window = planning_window(
                    self.model.as_ref(),
                    &self.budget,
                    plan_input.clone(),
                    &pending_originals,
                    &exact_sources,
                );
            }
            if window.is_err() && bytes(&last_plan) > self.data_limit() {
                plan_input["previous_plan"] =
                    json!({"read_ref":{"kind":"summary","id":"previous-plan"}});
                window = planning_window(
                    self.model.as_ref(),
                    &self.budget,
                    plan_input,
                    &pending_originals,
                    &exact_sources,
                );
            }
            let (plan_input, delivered_count, delivered_keys) = window?;
            let delivered: Vec<_> = pending_originals.drain(..delivered_count).collect();
            let reply = self.ask(lease, "plan", plan_input).await?;
            for r in self
                .confirm_planning_reads(lease, &delivered, &coverage, &mut saved)
                .await?
            {
                read_refs.insert(r.clone());
            }
            expand_interface_reads(snapshot, &mut read_refs);
            self.store.checkpoint(lease,&MaintenanceCheckpoint {
                id:"planning-progress".into(),phase:"planning".into(),references:vec![],review:None,summary:None,
                data:json!({"delivered_originals":delivered_count,"delivered_navigation":delivered_keys.len(),"queued_originals":pending_originals.len(),"queued_navigation":exact_sources.len()-delivered_keys.len()}),
            },&coverage).await?;
            let delivered_any = delivered_count > 0 || !delivered_keys.is_empty();
            // Navigation indexes were delivered verbatim in this successful planning call.
            // Do not keep them crowding out the original targets/evidence of later changes.
            for key in delivered_keys {
                exact_sources.remove(&key);
                if let Some(mut checkpoint) = saved.get(&key).cloned() {
                    checkpoint.phase = "index_delivered".into();
                    self.store.checkpoint(lease, &checkpoint, &coverage).await?;
                    saved.insert(key, checkpoint);
                }
            }
            let needed = match reply {
                MaintenanceReply::Read { requests } => requests
                    .into_iter()
                    .flat_map(|r| {
                        r.ids.into_iter().map(move |id| KnowledgeEvidenceRef {
                            kind: r.kind.clone(),
                            id,
                        })
                    })
                    .collect::<Vec<_>>(),
                MaintenanceReply::Plan { plan } => {
                    let checked =
                        materialize_maintenance(snapshot, &plan, lease.run.id, coverage.clone());
                    let failure = match &checked {
                        Err(Error::InvalidInput { message }) => Some(message.clone()),
                        Err(_) => return checked,
                        Ok(c) if !c.issues.is_empty() || !c.review.structurally_valid => {
                            Some(c.issues.join(","))
                        }
                        _ => None,
                    };
                    if let Some(code) = failure {
                        if validation_failures >= self.budget.retries {
                            return checked.and_then(|c| {
                                if c.issues.is_empty() {
                                    Ok(c)
                                } else {
                                    Err(invalid("CANDIDATE_VALIDATION_FAILED"))
                                }
                            });
                        }
                        validation_failures += 1;
                        validation_feedback = json!({"code":code,"instruction":"Repair the whole plan without changing scope silently. replace_catalog requires strategy full; insert preserves existing directories, classified memberships and existing merge groups. Changes to these use partial/full. Relations require captured link facts and correct current FieldRefs. Enum values/labels require corresponding facts; keep complete=false for observed values. Never remove observed values just because absent in recent samples."});
                        last_plan = encoded(&plan);
                        continue;
                    }
                    validation_feedback = Value::Null;
                    let needed = required_reads(snapshot, &plan);
                    let missing: Vec<_> = needed
                        .into_iter()
                        .filter(|r| !read_refs.contains(r))
                        .collect();
                    if missing.is_empty() && delivering_originals {
                        last_plan = encoded(&plan);
                        continue;
                    }
                    if missing.is_empty() {
                        let candidate = materialize_maintenance(
                            snapshot,
                            &plan,
                            lease.run.id,
                            coverage.clone(),
                        )?;
                        return Ok(candidate);
                    }
                    last_plan = encoded(&plan);
                    missing
                }
                _ => return Err(invalid("PLANNER_CONTRACT_INVALID")),
            };
            if needed.is_empty() {
                if delivered_any || !pending_originals.is_empty() || !exact_sources.is_empty() {
                    continue;
                }
                return Err(invalid("PLANNER_EMPTY_READ"));
            }
            let mut ordinary = Vec::new();
            for r in needed {
                if r.kind == "summary" || read_refs.contains(&r) {
                    let source = if r.kind == "summary" {
                        let source = saved.get(&r.id).ok_or(Error::NotFound)?;
                        if source.phase == "readback_complete" {
                            source.data["source"].clone()
                        } else if let Some(review) = &source.review {
                            json!({"review":review,"meaning":"Review conclusions are not independent evidence"})
                        } else if source.summary.is_none() {
                            source.data.clone()
                        } else {
                            json!({"summary":source.summary,"source_items":source.data.get("items"),"references":source.references,"meaning":"Navigation, not independent evidence"})
                        }
                    } else {
                        resource(snapshot, &r, &saved)?
                    };
                    let id = format!("navigation:{}:{}", r.kind, r.id);
                    // Preserve the established summary-navigation key for existing consumers.
                    let id = if r.kind == "summary" {
                        format!("navigation:{}", r.id)
                    } else {
                        id
                    };
                    self.save_navigation(lease, &id, source, &coverage, &mut saved)
                        .await?;
                    let mut checkpoint = saved[&id].clone();
                    let source = checkpoint.data.clone();
                    checkpoint.phase = "index_read".into();
                    checkpoint.references = vec![r];
                    checkpoint.data = json!({"source":source,"meaning":"Server prepared; not yet delivered to model"});
                    self.store.checkpoint(lease, &checkpoint, &coverage).await?;
                    exact_sources.insert(id.clone(), source);
                    saved.insert(id, checkpoint);
                    continue;
                }
                if !pending_originals
                    .iter()
                    .any(|item| item["reference"] == encoded(&r))
                {
                    ordinary.push(r);
                }
            }
            let (prepared, completed) = self
                .prepare_readbacks(lease, ordinary, &coverage, &mut saved)
                .await?;
            pending_originals.extend(prepared);
            for r in completed {
                read_refs.insert(r.clone());
            }
            expand_interface_reads(snapshot, &mut read_refs);
        }
        Err(invalid("MODEL_CALL_BUDGET_EXHAUSTED"))
    }
}

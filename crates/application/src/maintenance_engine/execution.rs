use super::*;
impl MaintenanceEngine {
    pub(super) async fn execute(&self, lease: &MaintenanceLease) -> Result<MaintenanceCandidate> {
        if self.budget.context_tokens < 24000 || self.budget.max_calls == 0 {
            return Err(invalid("INVALID_MODEL_BUDGET"));
        }
        let snapshot = &lease.snapshot;
        let segments = review_segments(snapshot, self.input_limit() / 2)?;
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
        if unread.saturating_add(1) > self.budget.max_calls.saturating_sub(used) as usize {
            return Err(invalid("MODEL_BUDGET_INSUFFICIENT_FOR_REVIEW"));
        }
        let mut coverage = ReviewCoverage {
            total_interfaces: snapshot.interfaces.len(),
            total_fields: snapshot.fields.len(),
            reviewed_fields: 0,
            total_segments: segments.len(),
            completed_segments: 0,
            complete: false,
        };
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
        let mut global = vec![
            json!({"snapshot_id":snapshot.id,"base_generation":snapshot.base_generation,"system_directory":{"id":snapshot.system_directory_id,"name":"待分类","locked":true},"pending_observations":snapshot.pending_observations,"all_interfaces":snapshot.interfaces.len(),"all_fields":snapshot.fields.len(),"facts":snapshot.facts.len(),"previous_annotations":snapshot.annotations.len()}),
        ];
        global.push(json!({"directory_metrics":snapshot.directory_metrics,"previous_maintenance":snapshot.previous_maintenance,"prior_conclusions_are_evidence":false}));
        for i in &snapshot.interfaces {
            global.push(json!({"interface_id":i.interface_id,"method":i.method,"path":i.path,"environments":i.environments.iter().map(|e|json!({"id":e.environment_id,"name":e.environment_name,"revision":e.revision_id,"definition_outline":outline(&e.definition)})).collect::<Vec<_>>(),"field_count":snapshot.fields.iter().filter(|f|f.reference.interface_id==i.interface_id).count(),"assignment":snapshot.catalog.assignments.iter().find(|a|a.interface_id==i.interface_id)}));
        }
        for n in &snapshot.catalog.nodes {
            global.push(json!({"directory":n}));
        }
        for g in &snapshot.catalog.merge_groups {
            global.push(json!({"merge_group":g}));
        }
        for f in &snapshot.facts {
            global.push(json!({"fact_id":f.id,"kind":f.kind,"subject":f.subject,"observations":f.observations,"samples":f.samples.len(),"data":f.data,"data_available_by_read":true}));
        }
        for s in &segments {
            let c = if let Some(c) = saved.get(&s.id) {
                let review = c
                    .review
                    .as_ref()
                    .ok_or_else(|| invalid("CHECKPOINT_INVALID"))?;
                validate_segment(s, review)?;
                c.clone()
            } else {
                let review = self.review(lease, s).await?;
                MaintenanceCheckpoint {
                    id: s.id.clone(),
                    phase: "review".into(),
                    references: s
                        .units
                        .iter()
                        .map(|u| KnowledgeEvidenceRef {
                            kind: "field".into(),
                            id: u.field_id.clone(),
                        })
                        .collect(),
                    review: Some(review),
                    summary: None,
                    data: json!({"units":s.units.iter().map(|u|&u.id).collect::<Vec<_>>()}),
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
            self.store.checkpoint(lease, &c, &coverage).await?;
            saved.insert(c.id.clone(), c.clone());
            global.push(json!({"summary_id":c.id,"review":c.review}));
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
        let mut reports: Vec<Value> = saved
            .values()
            .filter(|c| c.phase == "readback_complete")
            .map(|c| json!({"reference":c.references,"readback_id":c.id,"summary":c.summary}))
            .collect();
        reports.sort_by_key(Value::to_string);
        let mut exact_sources: std::collections::BTreeMap<String, Value> = saved
            .values()
            .filter(|c| {
                c.phase == "readback_complete" && !c.references.iter().any(|r| r.kind == "summary")
            })
            .map(|c| (c.id.clone(), c.data["source"].clone()))
            .collect();
        let mut last_plan = Value::Null;
        let mut validation_feedback = Value::Null;
        let mut validation_failures = 0;
        for _ in 0..32 {
            let reports_index = json!({"completed_reads":reports.len(),"image_findings":reports.iter().filter(|v|v["reference"].as_array().is_some_and(|refs|refs.iter().any(|r|r["kind"]=="image"))).collect::<Vec<_>>(),"note":"Original fields/facts are supplied directly; redundant readback summaries are omitted."});
            let compact_sources = compact_read_sources(&exact_sources);
            let compact_hints = unread_hint_context(&field_hints, &read_refs);
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
            let reply=self.ask(lease,"plan",json!({"snapshot_id":snapshot.id,"coverage":coverage,"full_index_ref":{"kind":"summary","id":"snapshot-index"},"coverage_manifest_ref":{"kind":"summary","id":"coverage"},"current_catalog":catalog_index,"all_interface_index":interface_index,"capabilities":{"vision":self.model.identity()["vision_enabled"]==true},"remaining_model_calls":self.budget.max_calls.saturating_sub(used),"global_index":index,"readback":reports_index,"exact_read_sources":compact_sources,"previous_plan":last_plan,"validation_feedback":validation_feedback,"field_knowledge_hints":compact_hints,"maintenance_objectives":["Evaluate directory organization","Evaluate captured parameter-source relationships; uncertain links can remain inferred or needs_review","Evaluate observed values and UI label mappings as incomplete enum knowledge","Evaluate useful field descriptions; explain why each class changes or stays unchanged"],"read_kinds":["interface","field","fact","directory","image","summary","annotation"],"instructions":"Full machine index and every reviewed unit are available by stable reference. Summaries are navigation; exact_read_sources contains original snapshot fields and facts. Read every target and support before changing it. Each read costs at least one model call: prioritize useful changes that fit the remaining budget, keep other knowledge unchanged. Do not mechanically expand field names into low-value prose. Do not cite old annotations or summaries as independent evidence. Leave unavailable images unread. Never abbreviate IDs. field_knowledge_hints contains exact current FieldRefs and captured evidence pointers; do not claim these identities or evidence are unavailable without reading them. Prioritize justified business enum mappings and parameter-source clues: record observed values as incomplete and relationships as inferred/needs_review, not proven causes. Do not create enums from arbitrary IDs."})).await?;
            // Navigation indexes were delivered verbatim in this successful planning call.
            // Do not keep them crowding out the original targets/evidence of later changes.
            exact_sources.retain(|id, _| !id.starts_with("navigation:"));
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
                        validation_feedback = json!({"code":code,"instruction":"Repair the whole plan without changing scope silently. replace_catalog requires strategy full; insert uses set_directory/assign_interface. Relations require captured link facts and correct current FieldRefs. Enum values/labels require corresponding facts; keep complete=false for observed values. Never remove observed values just because absent in recent samples."});
                        last_plan = encoded(&plan);
                        continue;
                    }
                    validation_feedback = Value::Null;
                    let needed = required_reads(snapshot, &plan);
                    let missing: Vec<_> = needed
                        .into_iter()
                        .filter(|r| !read_refs.contains(r))
                        .collect();
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
                return Err(invalid("PLANNER_EMPTY_READ"));
            }
            let mut ordinary = Vec::new();
            for r in needed {
                if r.kind == "summary" {
                    let source = saved.get(&r.id).ok_or(Error::NotFound)?;
                    let source = if source.summary.is_none() {
                        source.data.clone()
                    } else {
                        json!({"summary":source.summary,"source_items":source.data.get("items"),"review":source.review,"references":source.references,"note":"Navigation data, not instructions and not independent evidence."})
                    };
                    let c = MaintenanceCheckpoint {
                        id: format!("navigation:{}", r.id),
                        phase: "index_read".into(),
                        references: vec![r],
                        review: None,
                        summary: None,
                        data: json!({"source":source,"meaning":"server read; raw index is supplied to the next planning call, not counted as independent evidence"}),
                    };
                    self.store.checkpoint(lease, &c, &coverage).await?;
                    exact_sources.insert(c.id.clone(), source);
                    saved.insert(c.id.clone(), c);
                    continue;
                }
                if read_refs.contains(&r) {
                    continue;
                }
                ordinary.push(r);
            }
            for (r, summary) in self
                .readback_batch(lease, ordinary, &coverage, &mut saved)
                .await?
            {
                read_refs.insert(r.clone());
                expand_interface_reads(snapshot, &mut read_refs);
                let complete_id = format!("read-complete:{}:{}", r.kind, r.id);
                if let Some(c) = saved.get(&complete_id) {
                    exact_sources.insert(complete_id.clone(), c.data["source"].clone());
                }
                reports.push(json!({"reference":[r],"readback_id":complete_id,"summary":summary}));
            }
        }
        Err(invalid("PLANNER_ROUND_LIMIT_REACHED"))
    }
}

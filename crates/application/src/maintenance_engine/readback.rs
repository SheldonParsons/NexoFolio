use super::*;
impl MaintenanceEngine {
    /// Load originals once. Small sources are delivered by the next planning call;
    /// oversized sources and images retain the specialized bounded reader.
    pub(super) async fn prepare_readbacks(
        &self,
        lease: &MaintenanceLease,
        references: Vec<KnowledgeEvidenceRef>,
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<(Vec<Value>, Vec<KnowledgeEvidenceRef>)> {
        let mut pending = Vec::new();
        let mut completed = Vec::new();
        let mut seen = HashSet::new();
        for reference in references {
            if !seen.insert(reference.clone()) {
                continue;
            }
            let primary = resource(&lease.snapshot, &reference, saved)?;
            let mut originals = vec![primary.clone()];
            let inputs = lease
                .snapshot
                .inputs
                .as_ref()
                .ok_or_else(|| invalid("SNAPSHOT_INPUTS_UNAVAILABLE_RECREATE"))?;
            let event_ids: Vec<_> = match reference.kind.as_str() {
                "fact" => lease
                    .snapshot
                    .facts
                    .iter()
                    .find(|f| f.id.to_string() == reference.id)
                    .ok_or(Error::NotFound)?
                    .samples
                    .clone(),
                "source" => vec![reference.id.parse().map_err(|_| Error::NotFound)?],
                "observation" => inputs
                    .sources
                    .iter()
                    .filter(|s| {
                        s.ingestion_id
                            .is_some_and(|id| id.to_string() == reference.id)
                    })
                    .map(|s| s.event_id)
                    .collect(),
                _ => vec![],
            };
            for event in event_ids {
                let source = inputs
                    .sources
                    .iter()
                    .find(|s| s.event_id == event && s.project_id == lease.snapshot.project_id)
                    .ok_or_else(|| invalid("SOURCE_OUTSIDE_SNAPSHOT"))?;
                let original = self.sources.observation(source).await?;
                if reference.kind == "fact" {
                    // Verify pinned bytes, but do not attach every full capture body to a fact read.
                    // The model must explicitly request source to read those bodies.
                    originals.push(json!({"reference":{"kind":"source","id":source.event_id},"captured_at":source.captured_at,"raw_hash":source.raw_hash,"coverage":source.evidence_coverage,"contents_provided":false,"raw_hash_verified":true}));
                } else {
                    originals.push(original);
                }
            }
            let item = json!({"reference":reference,"originals":originals});
            if reference.kind == "image" || bytes(&item) > self.data_limit() {
                self.readback(lease, &reference, (primary, originals), coverage, saved)
                    .await?;
                completed.push(reference);
            } else {
                pending.push(item);
            }
        }
        Ok((pending, completed))
    }
    /// Called only after a valid response to the plan request containing these originals.
    pub(super) async fn confirm_planning_reads(
        &self,
        lease: &MaintenanceLease,
        originals: &[Value],
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<Vec<KnowledgeEvidenceRef>> {
        if originals.is_empty() {
            return Ok(vec![]);
        }
        let references = originals
            .iter()
            .map(|item| {
                serde_json::from_value(item["reference"].clone())
                    .map_err(|_| invalid("READBACK_CONTRACT_INVALID"))
            })
            .collect::<Result<Vec<KnowledgeEvidenceRef>>>()?;
        let checkpoint = MaintenanceCheckpoint {
            id: format!("planning-read:{}", digest(&json!(originals))),
            phase: "readback".into(),
            references: references.clone(),
            review: None,
            summary: None,
            data: json!({"via":"successful_planning_call","source_count":originals.len()}),
        };
        self.store.checkpoint(lease, &checkpoint, coverage).await?;
        saved.insert(checkpoint.id.clone(), checkpoint);
        for (reference, item) in references.iter().zip(originals) {
            self.complete_read(lease,reference,"Original delivered in a successful planning call",
                json!({"source":item["originals"][0],"provided_objects":item["originals"].as_array().map(Vec::len),"full_captures_provided":item["originals"].as_array().into_iter().flatten().filter(|v|v.get("event_id").is_some()&&v.get("original").is_some()).count(),"via":"planning"}),coverage,saved).await?;
        }
        Ok(references)
    }
    async fn readback(
        &self,
        lease: &MaintenanceLease,
        r: &KnowledgeEvidenceRef,
        source: (Value, Vec<Value>),
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<String> {
        let (primary, originals) = source;
        if r.kind == "image" {
            if self.model.identity()["vision_enabled"] != true {
                return Err(Error::NotConfigured {
                    capability: "maintenance_vision_readback",
                });
            }
            let image = self
                .sources
                .image(
                    lease.snapshot.project_id,
                    r.id.parse().map_err(|_| Error::NotFound)?,
                )
                .await?;
            let reply = self
                .ask(
                    lease,
                    "image_read",
                    json!({"reference":r,"context":originals,"image_data_url":image}),
                )
                .await?;
            let MaintenanceReply::Image {
                read: true,
                summary,
                limitations,
            } = reply
            else {
                return Err(invalid("IMAGE_NOT_READ"));
            };
            self.complete_read(
                lease,
                r,
                &summary,
                json!({"image_read":true,"limitations":limitations,"source":primary}),
                coverage,
                saved,
            )
            .await?;
            return Ok(summary);
        }
        let mut summaries = vec![];
        {
            let source_index = 0;
            let parts = fragment(&encoded(&originals), self.data_limit())?;
            for (part, entries) in parts.iter().enumerate() {
                let key = format!("read:{}:{}:{source_index}:{part}", r.kind, r.id);
                let c = if let Some(c) = saved.get(&key) {
                    c.clone()
                } else {
                    let MaintenanceReply::Summary{summary}=self.ask(lease,"readback",json!({"reference":r,"source":source_index,"part":part+1,"parts":parts.len(),"original_json_entries":entries,"instruction":"This is actual snapshot/source content, not a summary. Read all entries and retain support, counterexamples and uncertainty. JSON pointers reconstruct the original value."})).await? else{return Err(invalid("READBACK_CONTRACT_INVALID"))};
                    let c = MaintenanceCheckpoint {
                        id: key.clone(),
                        phase: "readback".into(),
                        references: vec![r.clone()],
                        review: None,
                        summary: Some(summary),
                        data: json!({"source":source_index,"part":part+1,"parts":parts.len()}),
                    };
                    self.store.checkpoint(lease, &c, coverage).await?;
                    saved.insert(key, c.clone());
                    c
                };
                summaries.push(json!({"readback_id":c.id,"summary":c.summary}));
            }
        }
        let index = self.global(lease, summaries, coverage, saved).await?;
        let summary = index.to_string();
        self.complete_read(
            lease,
            r,
            &summary,
            json!({"source":primary,"provided_objects":originals.len(),"full_captures_provided":originals.iter().filter(|v|v.get("event_id").is_some()&&v.get("original").is_some()).count()}),
            coverage,
            saved,
        )
        .await?;
        Ok(summary)
    }
    async fn complete_read(
        &self,
        lease: &MaintenanceLease,
        reference: &KnowledgeEvidenceRef,
        summary: &str,
        data: Value,
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<()> {
        let checkpoint = MaintenanceCheckpoint {
            id: format!("read-complete:{}:{}", reference.kind, reference.id),
            phase: "readback_complete".into(),
            references: vec![reference.clone()],
            review: None,
            summary: Some(summary.into()),
            data,
        };
        self.store.checkpoint(lease, &checkpoint, coverage).await?;
        saved.insert(checkpoint.id.clone(), checkpoint);
        Ok(())
    }
}

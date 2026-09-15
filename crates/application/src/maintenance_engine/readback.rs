use super::*;
impl MaintenanceEngine {
    pub(super) async fn readback_batch(
        &self,
        lease: &MaintenanceLease,
        references: Vec<KnowledgeEvidenceRef>,
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<Vec<(KnowledgeEvidenceRef, String)>> {
        let mut results = Vec::new();
        let mut pending = Vec::new();
        let mut seen = HashSet::new();
        for reference in references {
            if !seen.insert(reference.clone()) {
                continue;
            }
            let primary = resource(&lease.snapshot, &reference, saved)?;
            let mut originals = vec![primary.clone()];
            if reference.kind == "fact" {
                let fact = lease
                    .snapshot
                    .facts
                    .iter()
                    .find(|f| f.id.to_string() == reference.id)
                    .ok_or(Error::NotFound)?;
                for event in &fact.samples {
                    originals.push(
                        self.sources
                            .observation(lease.snapshot.project_id, *event)
                            .await?,
                    );
                }
            }
            let item = json!({"reference":reference,"source":primary,"originals":originals});
            if reference.kind == "image" || bytes(&item) > self.data_limit() {
                let summary = self.readback(lease, &reference, coverage, saved).await?;
                results.push((reference, summary));
            } else {
                pending.push(item);
            }
        }
        for group in groups(pending, self.data_limit())? {
            let MaintenanceReply::Summary { summary } = self.ask(lease, "readback", json!({
                "original_sources":group,
                "instruction":"Read every original source and reference in this batch. Preserve support, counterexamples and uncertainty. These are source records, not summaries."
            })).await? else { return Err(invalid("READBACK_CONTRACT_INVALID")); };
            let references = group
                .iter()
                .map(|item| {
                    serde_json::from_value(item["reference"].clone())
                        .map_err(|_| invalid("READBACK_CONTRACT_INVALID"))
                })
                .collect::<Result<Vec<KnowledgeEvidenceRef>>>()?;
            let batch_checkpoint = MaintenanceCheckpoint {
                id: format!("read-batch:{}", digest(&json!(group))),
                phase: "readback".into(),
                references,
                review: None,
                summary: Some(summary.clone()),
                data: json!({"batched":true,"source_count":group.len()}),
            };
            self.store
                .checkpoint(lease, &batch_checkpoint, coverage)
                .await?;
            saved.insert(batch_checkpoint.id.clone(), batch_checkpoint);
            // Mark each reference read only after the call containing all its original data succeeds.
            for item in group {
                let reference: KnowledgeEvidenceRef =
                    serde_json::from_value(item["reference"].clone())
                        .map_err(|_| invalid("READBACK_CONTRACT_INVALID"))?;
                let checkpoint = MaintenanceCheckpoint {
                    id: format!("read-complete:{}:{}", reference.kind, reference.id),
                    phase: "readback_complete".into(),
                    references: vec![reference.clone()],
                    review: None,
                    summary: Some(summary.clone()),
                    data: json!({"source":item["source"],"original_sources":item["originals"].as_array().map(Vec::len),"batched":true}),
                };
                self.store.checkpoint(lease, &checkpoint, coverage).await?;
                saved.insert(checkpoint.id.clone(), checkpoint);
                results.push((reference, summary.clone()));
            }
        }
        Ok(results)
    }
    async fn readback(
        &self,
        lease: &MaintenanceLease,
        r: &KnowledgeEvidenceRef,
        coverage: &ReviewCoverage,
        saved: &mut HashMap<String, MaintenanceCheckpoint>,
    ) -> Result<String> {
        let data = resource(&lease.snapshot, r, saved)?;
        let primary = data.clone();
        let mut originals = vec![data];
        if r.kind == "fact" {
            let f = lease
                .snapshot
                .facts
                .iter()
                .find(|f| f.id.to_string() == r.id)
                .ok_or(Error::NotFound)?;
            for event in &f.samples {
                originals.push(
                    self.sources
                        .observation(lease.snapshot.project_id, *event)
                        .await?,
                );
            }
        }
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
            let c = MaintenanceCheckpoint {
                id: format!("read-complete:image:{}", r.id),
                phase: "readback_complete".into(),
                references: vec![r.clone()],
                review: None,
                summary: Some(summary.clone()),
                data: json!({"image_read":true,"limitations":limitations,"source":primary}),
            };
            self.store.checkpoint(lease, &c, coverage).await?;
            saved.insert(c.id.clone(), c);
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
        let c = MaintenanceCheckpoint {
            id: format!("read-complete:{}:{}", r.kind, r.id),
            phase: "readback_complete".into(),
            references: vec![r.clone()],
            review: None,
            summary: Some(summary.clone()),
            data: json!({"source":primary,"original_sources":originals.len()}),
        };
        self.store.checkpoint(lease, &c, coverage).await?;
        saved.insert(c.id.clone(), c);
        Ok(summary)
    }
}

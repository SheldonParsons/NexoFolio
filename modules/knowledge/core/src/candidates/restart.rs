//! Automatic takeover is narrower than a user's explicit resume.
use super::{recovery, snapshot, validation::load};
use crate::{Clock, Knowledge, store_error};
use nexofolio_common::ProjectId;
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::KnowledgeStore;

fn blocked(candidate: &mut Candidate, message: &str, at: chrono::DateTime<chrono::Utc>) {
    candidate.state = CandidateState::Interrupted;
    candidate.updated_at = at;
    if !candidate.progress.is_object() {
        candidate.progress = serde_json::json!({});
    }
    candidate.progress["status"] = serde_json::json!("failed");
    candidate.progress["candidate_state"] = serde_json::json!("interrupted");
    candidate.progress["active"] = serde_json::json!([]);
    candidate.progress["resume_available"] = serde_json::json!(false);
    candidate.progress["recovery_blocked"] = serde_json::json!(true);
    candidate.progress["message"] = serde_json::json!(message);
    if let Some(attempt) = candidate
        .progress
        .get_mut("attempt_history")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|a| a.last_mut())
        .filter(|a| a.is_object() && a.get("ended_at").is_none_or(serde_json::Value::is_null))
    {
        attempt["ended_at"] = serde_json::json!(at);
        attempt["outcome"] = serde_json::json!("interrupted");
    }
}

impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn recover_proposal(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
    ) -> KnowledgeResult<Option<Candidate>> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = match load(tx.as_mut(), project, id).await {
            Ok(c) => c,
            Err(KnowledgeError::NoSuchCandidate) => return Ok(None),
            Err(e) => return Err(e),
        };
        let now = self.clock.now();
        // The scan is only a hint. A manual stop or fresh heartbeat wins here.
        if candidate.state != CandidateState::Running
            || now - candidate.updated_at < chrono::TimeDelta::seconds(recovery::LEASE_SECONDS)
        {
            return Ok(None);
        }
        let issue = if candidate.execution_input.is_none() {
            Some("服务已重启，但本轮尚未保存完整恢复资料；已保存内容保留，请开始新一轮整理")
        } else {
            let endpoints = self
                .endpoints
                .list(project)
                .await
                .map_err(|_| KnowledgeError::Unavailable)?
                .into_iter()
                .map(|e| e.id)
                .collect();
            let current = snapshot::capture(tx.as_mut(), project, endpoints).await?;
            (snapshot::hash(&current) != candidate.base_hash).then_some(
                "服务已重启，但正式文档已变化，无法自动续跑本轮；已保存内容保留，请审阅或重新整理",
            )
        };
        if let Some(message) = issue {
            blocked(&mut candidate, message, now);
            tx.put_candidate(&candidate).await.map_err(store_error)?;
            tx.commit().await.map_err(store_error)?;
            return Ok(None);
        }
        // Also refuses another live proposal in this project. Any temporary
        // changes made by available are rolled back when that refusal occurs.
        match recovery::available(tx.as_mut(), project, now).await {
            Err(KnowledgeError::CandidateBusy) => return Ok(None),
            Err(e) => return Err(e),
            Ok(()) => {}
        }
        recovery::claim(
            &mut candidate,
            owner,
            CandidateActor::System {
                id: "curation-recovery".into(),
                display_name: "系统".into(),
            },
            now,
        );
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(candidate))
    }
}

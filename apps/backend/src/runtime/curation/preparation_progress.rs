//! Preparation facts visible before any model request; never includes sample values.
use super::{activity::Activity, jobs::Job};
use chrono::{DateTime, Utc};
use nexofolio_curate::Pass;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PreparationPhase {
    Reading,
    Analyzing,
    SavingValues,
    SavingInput,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparationProgress {
    pub phase: PreparationPhase,
    pub phase_started_at: DateTime<Utc>,
    pub endpoints_read: usize,
    pub endpoints_total: usize,
    pub samples_read: usize,
    pub samples_unavailable: usize,
    pub fields: usize,
    pub values: usize,
    pub relations: usize,
}

impl Job {
    pub(super) fn prepare_start(&self, total: usize) {
        self.update(|p| p.resume_available = Some(false));
        self.update(|p| {
            p.preparation = Some(PreparationProgress {
                phase: PreparationPhase::Reading,
                phase_started_at: Utc::now(),
                endpoints_read: 0,
                endpoints_total: total,
                samples_read: 0,
                samples_unavailable: 0,
                fields: 0,
                values: 0,
                relations: 0,
            })
        });
        self.prepare_phase(PreparationPhase::Reading, "正在读取真实样本与字段结构");
    }

    pub(super) fn prepare_phase(&self, phase: PreparationPhase, message: &str) {
        self.update(|p| {
            if let Some(preparation) = &mut p.preparation {
                preparation.phase = phase;
                preparation.phase_started_at = Utc::now();
            }
            p.message = Some(message.into());
        });
        tracing::info!(?phase, "curation preparation phase");
        self.record(
            Activity::new(Pass::Describe, "prepare", message).name(Some("整理准备".into())),
        );
    }
}

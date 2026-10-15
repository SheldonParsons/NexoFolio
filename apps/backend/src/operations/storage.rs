//! Reuses the operations clock and Snapshot contract; no cleanup in the sampler.
use super::snapshot::Snapshot;
use chrono::Utc;
use nexofolio_common::{
    ProjectId,
    storage::{StorageReader, StorageUsage},
};
use nexofolio_knowledge_contracts::retention::{HistoryMaintenance, RetentionStatus};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone, Serialize)]
pub(crate) struct StorageSnapshot {
    pub project: ProjectId,
    pub modules: Vec<Snapshot<StorageUsage>>,
    pub retention: Snapshot<RetentionStatus>,
}
struct Entry {
    value: StorageSnapshot,
    requested: Instant,
    pending: bool,
}
#[derive(Clone)]
pub(crate) struct StorageMonitor {
    cache: Arc<Mutex<VecDeque<Entry>>>,
    readers: Vec<Arc<dyn StorageReader>>,
    pub maintenance: Arc<dyn HistoryMaintenance>,
    pub execution_allowed: bool,
    pub execution: Arc<tokio::sync::Mutex<()>>,
}
impl StorageMonitor {
    pub fn new(
        readers: Vec<Arc<dyn StorageReader>>,
        maintenance: Arc<dyn HistoryMaintenance>,
    ) -> Self {
        Self {
            cache: Arc::default(),
            readers,
            maintenance,
            execution_allowed: std::env::var("NEXOFOLIO_RETENTION_ENABLED").as_deref() == Ok("1"),
            execution: Arc::default(),
        }
    }
    fn empty(project: ProjectId) -> StorageSnapshot {
        StorageSnapshot {
            project,
            modules: vec![
                Snapshot::new(
                    "observe：样本、声明与代表值；项目完整聚合，超时保留旧快照",
                    "选定项目；物理表为全库共享",
                    60,
                ),
                Snapshot::new(
                    "knowledge：快照、归档、历史和正式说明；项目完整聚合，超时保留旧快照",
                    "选定项目；物理表为全库共享",
                    60,
                ),
            ],
            retention: Snapshot::new(
                "knowledge：启用时间与最多20个停止但可恢复任务",
                "选定项目",
                60,
            ),
        }
    }
    pub fn read(&self, project: ProjectId) -> StorageSnapshot {
        self.cache
            .lock()
            .ok()
            .and_then(|q| {
                q.iter()
                    .find(|e| e.value.project == project)
                    .map(|e| e.value.clone())
            })
            .unwrap_or_else(|| Self::empty(project))
    }
    pub fn request(&self, project: ProjectId) {
        if let Ok(mut q) = self.cache.lock() {
            if let Some(e) = q.iter_mut().find(|e| e.value.project == project) {
                if e.requested.elapsed() >= Duration::from_secs(15) {
                    e.pending = true;
                    e.requested = Instant::now();
                }
            } else {
                if q.len() == 32 {
                    q.pop_front();
                }
                q.push_back(Entry {
                    value: Self::empty(project),
                    requested: Instant::now(),
                    pending: true,
                });
            }
        }
    }
    pub fn invalidate(&self, project: ProjectId) {
        if let Ok(mut q) = self.cache.lock()
            && let Some(e) = q.iter_mut().find(|e| e.value.project == project)
        {
            e.pending = true;
            e.requested = Instant::now();
            e.value.retention.error = Some("维护后等待重新采样");
            for sample in &mut e.value.modules {
                sample.error = Some("维护后等待重新采样");
            }
        }
    }
    pub async fn sample_next(&self) {
        let next = self.cache.lock().ok().and_then(|mut q| {
            q.iter_mut().find(|e| e.pending).map(|e| {
                e.pending = false;
                (e.value.project, e.requested)
            })
        });
        let Some((project, generation)) = next else {
            return;
        };
        let mut value = Self::empty(project);
        for (reader, snapshot) in self.readers.iter().zip(value.modules.iter_mut()) {
            let result =
                tokio::time::timeout(Duration::from_secs(2), reader.storage_usage(project)).await;
            snapshot.record(
                result.ok().and_then(Result::ok).ok_or("采样超时或暂不可用"),
                Utc::now(),
            );
        }
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            self.maintenance.retention_status(project),
        )
        .await;
        value.retention.record(
            result.ok().and_then(Result::ok).ok_or("保护状态暂不可用"),
            Utc::now(),
        );
        if let Ok(mut q) = self.cache.lock()
            && let Some(e) = q
                .iter_mut()
                .find(|e| e.value.project == project && e.requested == generation)
        {
            for (old, new) in e.value.modules.iter_mut().zip(value.modules) {
                old.record(new.value.ok_or("采样超时或暂不可用"), Utc::now());
            }
            e.value
                .retention
                .record(value.retention.value.ok_or("保护状态暂不可用"), Utc::now());
        }
    }
}

//! Best-effort bounded audit: no database await on the request path.
pub(crate) mod console;
mod diagnostic_task;
mod diagnostics;
pub(crate) mod http;
pub(crate) mod routes;
#[cfg(test)]
mod tests;
pub(crate) use http::observe;
use nexofolio_audit_contracts::{AuditStore, Event, Limits};
use nexofolio_common::{Error, Result};
use std::{
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct Settings {
    pub enabled: bool,
    pub limits: Limits,
    pub trusted_proxies: Vec<IpAddr>,
}
impl Settings {
    pub fn parse(lookup: &impl Fn(&str) -> Option<String>) -> Result<Self> {
        let number = |name: &str, default: u32, max: u32| -> Result<u32> {
            let n = lookup(name)
                .map(|s| s.parse::<u32>())
                .transpose()
                .map_err(|_| Error::invalid(format!("{name} is invalid")))?
                .unwrap_or(default);
            if n == 0 || n > max {
                return Err(Error::invalid(format!("{name} is invalid")));
            }
            Ok(n)
        };
        let enabled = match lookup("NEXOFOLIO_AUDIT_ENABLED").as_deref() {
            None | Some("true") => true,
            Some("false") => false,
            _ => return Err(Error::invalid("NEXOFOLIO_AUDIT_ENABLED is invalid")),
        };
        let trusted_proxies = lookup("NEXOFOLIO_AUDIT_TRUSTED_PROXIES")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().parse())
            .collect::<std::result::Result<Vec<IpAddr>, _>>()
            .map_err(|_| {
                Error::invalid("NEXOFOLIO_AUDIT_TRUSTED_PROXIES must be exact IP addresses")
            })?;
        Ok(Self {
            enabled,
            trusted_proxies,
            limits: Limits {
                events: number("NEXOFOLIO_AUDIT_MAX_EVENTS", 100_000, 1_000_000)?,
                buckets: number("NEXOFOLIO_AUDIT_MAX_BUCKETS", 100_000, 1_000_000)?,
                operation_days: number("NEXOFOLIO_AUDIT_OPERATION_DAYS", 90, 365)?,
                error_days: number("NEXOFOLIO_AUDIT_ERROR_DAYS", 30, 90)?,
                access_days: number("NEXOFOLIO_AUDIT_ACCESS_DAYS", 30, 90)?,
            },
        })
    }
}
#[derive(Default)]
struct Health {
    dropped_access: AtomicU64,
    dropped_important: AtomicU64,
    storage_failures: AtomicU64,
    written: AtomicU64,
}
#[derive(Clone)]
pub struct Audit {
    access: mpsc::Sender<Event>,
    important: mpsc::Sender<Event>,
    health: Arc<Health>,
    pub settings: Settings,
}
impl Audit {
    pub fn start(
        store: Arc<dyn AuditStore>,
        settings: Settings,
        stop: CancellationToken,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        let (access, access_rx) = mpsc::channel(4096);
        let (important, important_rx) = mpsc::channel(1024);
        let audit = Self {
            access,
            important,
            health: Arc::default(),
            settings,
        };
        let worker = tokio::spawn(run(audit.clone(), store, access_rx, important_rx, stop));
        (audit, worker)
    }
    pub fn emit(&self, event: Event) {
        if !self.settings.enabled {
            return;
        }
        let ordinary = matches!(event.kind.as_str(), "page" | "api");
        let sender = if ordinary {
            &self.access
        } else {
            &self.important
        };
        if sender.try_send(event).is_err() {
            let counter = if ordinary {
                &self.health.dropped_access
            } else {
                &self.health.dropped_important
            };
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn health(&self) -> serde_json::Value {
        serde_json::json!({"enabled":self.settings.enabled,
            "dropped_access":self.health.dropped_access.load(Ordering::Relaxed),
            "dropped_important":self.health.dropped_important.load(Ordering::Relaxed),
            "storage_failures":self.health.storage_failures.load(Ordering::Relaxed),
            "written":self.health.written.load(Ordering::Relaxed),
            "queued_access":self.access.max_capacity()-self.access.capacity(),
            "queued_important":self.important.max_capacity()-self.important.capacity()})
    }
}
async fn run(
    audit: Audit,
    store: Arc<dyn AuditStore>,
    mut access: mpsc::Receiver<Event>,
    mut important: mpsc::Receiver<Event>,
    stop: CancellationToken,
) {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_purge = std::time::Instant::now() - Duration::from_secs(60);
    loop {
        tokio::select! { _ = tick.tick() => {}, _ = stop.cancelled() => { access.close(); important.close(); } }
        if !audit.settings.enabled {
            if stop.is_cancelled() {
                break;
            } else {
                continue;
            }
        }
        let mut batch = Vec::with_capacity(256);
        // Reserved critical queue, with at least 64 slots for access under sustained load.
        for _ in 0..192 {
            if let Ok(event) = important.try_recv() {
                batch.push(event);
            } else {
                break;
            }
        }
        while batch.len() < 256 {
            if let Ok(event) = access.try_recv() {
                batch.push(event);
            } else {
                break;
            }
        }
        while batch.len() < 256 {
            if let Ok(event) = important.try_recv() {
                batch.push(event);
            } else {
                break;
            }
        }
        if !batch.is_empty() {
            match tokio::time::timeout(
                Duration::from_secs(3),
                store.write(&batch, &audit.settings.limits),
            )
            .await
            {
                Ok(Ok(())) => {
                    audit
                        .health
                        .written
                        .fetch_add(batch.len() as u64, Ordering::Relaxed);
                    if access.len() >= 256 || !important.is_empty() {
                        tick.reset_immediately();
                    }
                }
                _ => {
                    audit
                        .health
                        .storage_failures
                        .fetch_add(1, Ordering::Relaxed);
                    for e in &batch {
                        let c = if matches!(e.kind.as_str(), "api" | "page") {
                            &audit.health.dropped_access
                        } else {
                            &audit.health.dropped_important
                        };
                        c.fetch_add(1, Ordering::Relaxed);
                    }
                    tracing::warn!(lost_events = batch.len(), "audit_batch_lost");
                }
            }
        }
        if !stop.is_cancelled() && last_purge.elapsed() >= Duration::from_secs(60) {
            last_purge = std::time::Instant::now();
            if !matches!(
                tokio::time::timeout(Duration::from_secs(3), store.purge(&audit.settings.limits))
                    .await,
                Ok(Ok(()))
            ) {
                audit
                    .health
                    .storage_failures
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!("audit_retention_failed");
            }
            let lost = audit.health.dropped_access.load(Ordering::Relaxed)
                + audit.health.dropped_important.load(Ordering::Relaxed);
            if lost > 0 {
                tracing::warn!(dropped_total = lost, "audit_degraded");
            }
        }
        if stop.is_cancelled() {
            if access.is_empty() && important.is_empty() {
                break;
            }
            tick.reset_immediately();
        }
    }
}

#[derive(Clone)]
pub(crate) struct Context {
    pub audit: Audit,
    event: Arc<Mutex<Event>>,
}
impl Context {
    pub(crate) fn execution(
        audit: Audit,
        task: &nexofolio_contracts::knowledge::CurationTask,
    ) -> Self {
        let ctx = Self {
            audit,
            event: Arc::new(Mutex::new(Event {
                execution: None,
                at: chrono::Utc::now(),
                kind: "operation".into(),
                action: String::new(),
                actor: task
                    .actor
                    .user_id()
                    .map(|id| format!("user:{id}"))
                    .unwrap_or_else(|| "unknown".into()),
                route: "/v1/projects/{project_id}/curate/tasks/{task_id}".into(),
                method: "WORKER".into(),
                status: 0,
                duration_ms: 0,
                request_id: String::new(),
                project: task.project_id.to_string(),
                resource: task.candidate_id.unwrap_or(task.id).to_string(),
                code: String::new(),
            })),
        };
        ctx.associate_task(task);
        ctx
    }
    pub(crate) fn associate_task(&self, task: &nexofolio_contracts::knowledge::CurationTask) {
        self.event
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execution = Some(nexofolio_audit_contracts::Execution {
            task_id: task.id,
            run_id: task.run_id,
            generation: task.generation,
        });
    }

    pub fn identify(&self, user: impl std::fmt::Display) {
        self.event.lock().unwrap_or_else(|e| e.into_inner()).actor = format!("user:{user}");
    }
    pub fn scope(&self, project: Option<uuid::Uuid>, resource: Option<uuid::Uuid>) {
        let mut event = self.event.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(id) = project {
            event.project = id.to_string();
        }
        if let Some(id) = resource {
            event.resource = id.to_string();
        }
    }
    pub fn event(&self) -> Event {
        self.event.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn record(
        &self,
        kind: &str,
        action: &str,
        code: &str,
        resource: Option<String>,
        duration_ms: u32,
    ) {
        let mut e = self.event();
        e.at = chrono::Utc::now();
        e.kind = kind.into();
        e.action = action.into();
        e.code = code.into();
        e.duration_ms = duration_ms;
        if let Some(id) = resource {
            e.resource = id;
        }
        self.audit.emit(e);
    }
}

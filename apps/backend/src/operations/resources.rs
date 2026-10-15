use super::snapshot::Snapshot;
use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Clone, Serialize)]
pub(crate) struct Resources {
    pub rss_bytes: Snapshot<u64>,
    pub cpu_cores: Snapshot<f32>,
    pub container_limit_bytes: Snapshot<Option<u64>>,
    pub host_available_bytes: Snapshot<u64>,
}
pub(super) struct Sampler {
    system: System,
    pid: Pid,
    previous: bool,
    values: Resources,
}
impl Sampler {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            pid: Pid::from_u32(std::process::id()),
            previous: false,
            values: Resources {
                rss_bytes: Snapshot::new("sysinfo.process.memory", "api_process", 20),
                cpu_cores: Snapshot::new(
                    "sysinfo.process.cpu_usage / 100",
                    "api_process; one full core = 1.0",
                    20,
                ),
                container_limit_bytes: Snapshot::new(
                    "cgroup memory hierarchy minimum; null value = unlimited",
                    "visible_cgroup_ancestry",
                    20,
                ),
                host_available_bytes: Snapshot::new(
                    "no verified host-availability collector",
                    "host (not process or container)",
                    20,
                ),
            },
        }
    }
    pub fn sample(&mut self) -> Resources {
        let now = chrono::Utc::now();
        let refreshed = self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        if let Some(process) = self
            .system
            .process(self.pid)
            .filter(|process| refreshed > 0 && process.memory() > 0)
        {
            self.values.rss_bytes.record(
                (process.memory() > 0)
                    .then_some(process.memory())
                    .ok_or("process_memory_unavailable"),
                now,
            );
            self.values.cpu_cores.record(
                if self.previous {
                    Ok(process.cpu_usage() / 100.0)
                } else {
                    Err("needs_second_sample")
                },
                now,
            );
            self.previous = true;
        } else {
            self.values
                .rss_bytes
                .record(Err("process_unavailable"), now);
            self.values
                .cpu_cores
                .record(Err("process_unavailable"), now);
            self.previous = false;
        }
        #[cfg(target_os = "linux")]
        self.values
            .container_limit_bytes
            .record(super::cgroup::limit(), now);
        #[cfg(not(target_os = "linux"))]
        self.values
            .container_limit_bytes
            .record(Err("not_a_linux_cgroup"), now);
        // sysinfo's host availability estimate has no per-refresh error signal and can
        // saturate to zero on macOS. Do not promote that ambiguous zero to a measurement.
        self.values
            .host_available_bytes
            .record(Err("host_available_not_verified"), now);
        self.values.clone()
    }
}

use super::{cgroup, resources::Sampler, snapshot::Snapshot};

#[test]
fn failed_samples_preserve_value_time_and_unknown_is_not_zero() {
    let mut s = Snapshot::<u64>::new("fixture", "process", 20);
    let before = chrono::Utc::now();
    s.record(Err("offline"), before);
    assert!(s.value.is_none());
    assert!(s.sampled_at.is_none());
    s.record(Ok(42), before);
    let after = before + chrono::TimeDelta::seconds(30);
    s.record(Err("offline"), after);
    assert_eq!(s.value, Some(42));
    assert_eq!(s.sampled_at, Some(before));
    assert_eq!(s.attempted_at, Some(after));
    assert_eq!(s.error, Some("offline"));
}
#[test]
fn cgroup_limits_keep_unlimited_unknown_nested_and_namespaced_roots_distinct() {
    assert_eq!(cgroup::parse_limit("max\n", true), Ok(None));
    assert_eq!(cgroup::parse_limit("9223372036854771712", false), Ok(None));
    assert_eq!(cgroup::parse_limit("0", true), Ok(Some(0)));
    assert_eq!(cgroup::parse_limit("1048576", true), Ok(Some(1048576)));
    assert!(cgroup::parse_limit("", true).is_err());
    assert!(cgroup::parse_limit("garbage", false).is_err());
    let mounts = "29 22 0:26 / /sys/fs/cgroup rw - cgroup2 cgroup rw";
    let (root, leaf, v2) = cgroup::locate("0::/slice/app", mounts).unwrap();
    assert_eq!(root.to_str(), Some("/sys/fs/cgroup"));
    assert_eq!(leaf.to_str(), Some("/sys/fs/cgroup/slice/app"));
    assert!(v2);
    let mounts = "29 22 0:26 /docker/abc /sys/fs/cgroup rw - cgroup2 cgroup rw";
    assert_eq!(
        cgroup::locate("0::/", mounts).unwrap().1,
        std::path::PathBuf::from("/sys/fs/cgroup")
    );
    let mounts = "29 22 0:26 / /sys/fs/cgroup/memory rw - cgroup cgroup rw,memory";
    assert!(!cgroup::locate("7:memory:/docker/a", mounts).unwrap().2);
    assert!(cgroup::locate("0::/../../escape", mounts).is_none());
}
#[test]
fn actual_process_resources_have_rss_and_cpu_requires_a_second_sample() {
    let mut sampler = Sampler::new();
    let first = sampler.sample();
    assert!(first.rss_bytes.value.is_some_and(|v| v > 0));
    assert_eq!(first.cpu_cores.value, None);
    assert_eq!(first.cpu_cores.error, Some("needs_second_sample"));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let next = sampler.sample();
    assert!(next.cpu_cores.value.is_some_and(|v| v >= 0.0));
    #[cfg(target_os = "macos")]
    {
        assert_eq!(next.host_available_bytes.value, None);
        assert_eq!(
            next.host_available_bytes.error,
            Some("host_available_not_verified")
        );
        assert_eq!(next.container_limit_bytes.sampled_at, None);
        assert_eq!(next.container_limit_bytes.error, Some("not_a_linux_cgroup"));
    }
}

#[test]
fn cgroup_files_preserve_ancestor_limit_unlimited_and_failure() {
    let root = std::env::temp_dir().join(format!("nexofolio-cgroup-{}", uuid::Uuid::new_v4()));
    let leaf = root.join("child");
    std::fs::create_dir_all(&leaf).unwrap();
    std::fs::write(root.join("memory.max"), "1048576").unwrap();
    std::fs::write(leaf.join("memory.max"), "2097152").unwrap();
    assert_eq!(cgroup::read_limits(&root, &leaf, true), Ok(Some(1048576)));
    std::fs::write(root.join("memory.max"), "max").unwrap();
    std::fs::write(leaf.join("memory.max"), "max").unwrap();
    assert_eq!(cgroup::read_limits(&root, &leaf, true), Ok(None));
    std::fs::write(leaf.join("memory.max"), "invalid").unwrap();
    assert_eq!(
        cgroup::read_limits(&root, &leaf, true),
        Err("cgroup_invalid")
    );
    std::fs::remove_file(leaf.join("memory.max")).unwrap();
    assert_eq!(
        cgroup::read_limits(&root, &leaf, true),
        Err("cgroup_read_failed")
    );
    std::fs::remove_dir_all(root).unwrap();
    #[cfg(target_os = "macos")]
    assert_eq!(cgroup::limit(), Err("cgroup_unavailable"));
}

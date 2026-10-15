//! Read the current process's visible cgroup ancestry; never infer host memory from a container.
use std::path::{Path, PathBuf};

pub(super) fn limit() -> Result<Option<u64>, &'static str> {
    let membership =
        std::fs::read_to_string("/proc/self/cgroup").map_err(|_| "cgroup_unavailable")?;
    let mounts =
        std::fs::read_to_string("/proc/self/mountinfo").map_err(|_| "cgroup_unavailable")?;
    let (root, leaf, v2) = locate(&membership, &mounts).ok_or("cgroup_unavailable")?;
    read_limits(&root, &leaf, v2)
}
pub(super) fn read_limits(root: &Path, leaf: &Path, v2: bool) -> Result<Option<u64>, &'static str> {
    let mut limits = Vec::new();
    for path in leaf.ancestors() {
        if !path.starts_with(root) {
            break;
        }
        let text = std::fs::read_to_string(path.join(if v2 {
            "memory.max"
        } else {
            "memory.limit_in_bytes"
        }));
        // A cgroup v2 hierarchy root has no memory.max. Other missing files are unknown.
        if path == root
            && v2
            && text
                .as_ref()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            break;
        }
        let value = parse_limit(&text.map_err(|_| "cgroup_read_failed")?, v2)?;
        if let Some(value) = value {
            limits.push(value);
        }
    }
    Ok(limits.into_iter().min())
}
pub(super) fn parse_limit(raw: &str, v2: bool) -> Result<Option<u64>, &'static str> {
    if v2 && raw.trim() == "max" {
        return Ok(None);
    }
    let value = raw.trim().parse::<u64>().map_err(|_| "cgroup_invalid")?;
    Ok((v2 || value < 0x7fff_ffff_ffff_f000).then_some(value))
}
fn unescape(path: &str) -> String {
    path.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\134", "\\")
}
pub(super) fn locate(membership: &str, mounts: &str) -> Option<(PathBuf, PathBuf, bool)> {
    for line in membership.lines() {
        let fields: Vec<_> = line.splitn(3, ':').collect();
        if fields.len() != 3 {
            continue;
        }
        let v2 = fields[1].is_empty();
        if !v2 && !fields[1].split(',').any(|v| v == "memory") {
            continue;
        }
        for mount in mounts.lines() {
            let (left, right) = mount.split_once(" - ")?;
            let parts: Vec<_> = left.split_whitespace().collect();
            let kind: Vec<_> = right.split_whitespace().collect();
            if parts.len() < 5 || kind.len() < 3 || kind[0] != if v2 { "cgroup2" } else { "cgroup" }
            {
                continue;
            }
            if !v2 && !kind[2].split(',').any(|v| v == "memory") {
                continue;
            }
            let member = Path::new(fields[2]);
            let base = PathBuf::from(unescape(parts[3]));
            let root = PathBuf::from(unescape(parts[4]));
            let relative = member
                .strip_prefix(&base)
                .ok()
                .or_else(|| (member == Path::new("/")).then_some(Path::new("")))?;
            if relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                continue;
            }
            return Some((root.clone(), root.join(relative), v2));
        }
    }
    None
}

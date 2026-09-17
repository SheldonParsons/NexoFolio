use nexofolio_contracts::*;
use std::collections::{HashMap, HashSet};

pub trait DirectoryReviewer: Send + Sync {
    fn review(&self, snapshot: &CatalogSnapshot, candidate: &DirectoryCandidate) -> PreviewReview;
}

/// Deterministic structural checks, independent of the model and its prompt.
pub struct StructuralDirectoryReviewer;
impl DirectoryReviewer for StructuralDirectoryReviewer {
    fn review(&self, snapshot: &CatalogSnapshot, candidate: &DirectoryCandidate) -> PreviewReview {
        let mut issues = Vec::new();
        let mut warnings = Vec::new();
        let mut issue = |code: &str, subject: String| {
            issues.push(PreviewIssue {
                code: code.into(),
                subject: Some(subject),
            })
        };
        let mut nodes = HashMap::new();
        let mut siblings = HashSet::new();
        if candidate.nodes.len() > 500 {
            issue("TOO_MANY_DIRECTORIES", candidate.nodes.len().to_string());
        }
        for node in &candidate.nodes {
            if node.name == "待分类" {
                issue("RESERVED_SYSTEM_DIRECTORY", node.id.to_string());
            }
            if nodes.insert(node.id, node).is_some() {
                issue("DUPLICATE_DIRECTORY_ID", node.id.to_string());
            }
            if node.name.trim().is_empty()
                || node.name != node.name.trim()
                || node.name.chars().count() > 80
                || node.name.chars().any(char::is_control)
            {
                issue("INVALID_DIRECTORY_NAME", node.id.to_string());
            }
            if node.description.chars().count() > 1000 {
                issue("DESCRIPTION_TOO_LONG", node.id.to_string());
            }
            if !siblings.insert((node.parent, node.name.trim().to_lowercase())) {
                issue("DUPLICATE_SIBLING_NAME", node.id.to_string());
            }
        }
        let mut max_depth = 0;
        for node in &candidate.nodes {
            let mut seen = HashSet::new();
            let mut current = Some(node.id);
            let mut depth = 0;
            while let Some(id) = current {
                if !seen.insert(id) {
                    issue("DIRECTORY_CYCLE", node.id.to_string());
                    break;
                }
                let Some(parent) = nodes.get(&id) else {
                    issue("UNKNOWN_PARENT", node.id.to_string());
                    break;
                };
                depth += 1;
                current = parent.parent;
            }
            max_depth = max_depth.max(depth);
        }
        let expected: HashSet<_> = snapshot.interfaces.iter().map(|i| i.interface_id).collect();
        let mut seen = HashSet::new();
        let mut assigned = HashSet::new();
        let mut unclassified = HashSet::new();
        let mut counts = HashMap::<DirectoryId, HashSet<InterfaceId>>::new();
        for item in &candidate.assignments {
            if !expected.contains(&item.interface_id) {
                issue("UNKNOWN_INTERFACE", item.interface_id.to_string());
                continue;
            }
            if !seen.insert(item.interface_id) {
                issue("DUPLICATE_ASSIGNMENT", item.interface_id.to_string());
            }
            if item.reason.trim().is_empty() || item.reason.chars().count() > 1000 {
                issue("INVALID_ASSIGNMENT_REASON", item.interface_id.to_string());
            }
            if let Some(id) = item.directory_id {
                if !nodes.contains_key(&id) {
                    issue("UNKNOWN_DIRECTORY", item.interface_id.to_string());
                } else {
                    assigned.insert(item.interface_id);
                    counts.entry(id).or_default().insert(item.interface_id);
                }
            } else {
                unclassified.insert(item.interface_id);
            }
        }
        for missing in expected.difference(&seen) {
            issue("MISSING_INTERFACE", missing.to_string());
        }
        let interfaces: HashMap<_, _> = snapshot
            .interfaces
            .iter()
            .map(|i| (i.interface_id, i))
            .collect();
        let assigned_to: HashMap<_, _> = candidate
            .assignments
            .iter()
            .map(|a| (a.interface_id, a.directory_id))
            .collect();
        let mut grouped = HashSet::new();
        let mut reduction = 0usize;
        for group in &candidate.merge_groups {
            let subject = group.representative_id.to_string();
            let unique: HashSet<_> = group.member_ids.iter().copied().collect();
            if unique.len() < 2
                || unique.len() != group.member_ids.len()
                || !unique.contains(&group.representative_id)
            {
                issue("INVALID_MERGE_MEMBERS", subject.clone());
            }
            if group.reason.trim().is_empty() || group.reason.chars().count() > 1000 {
                issue("INVALID_MERGE_REASON", subject.clone());
            }
            let representative = interfaces.get(&group.representative_id);
            for id in &group.member_ids {
                if !grouped.insert(*id) {
                    issue("OVERLAPPING_MERGE_GROUPS", id.to_string());
                }
                match (interfaces.get(id), representative) {
                    (Some(member), Some(rep)) => {
                        if member.method != rep.method {
                            issue("MERGE_METHOD_MISMATCH", id.to_string());
                        }
                        if !template_covers(&group.path_template, &member.path) {
                            issue("MERGE_TEMPLATE_MISMATCH", id.to_string());
                        }
                        if assigned_to.get(id) != assigned_to.get(&rep.interface_id) {
                            issue("MERGE_DIRECTORY_MISMATCH", id.to_string());
                        }
                    }
                    _ => issue("UNKNOWN_MERGE_MEMBER", id.to_string()),
                }
            }
            reduction += unique.len().saturating_sub(1);
        }
        let parents: HashSet<_> = candidate.nodes.iter().filter_map(|n| n.parent).collect();
        for node in &candidate.nodes {
            if !counts.contains_key(&node.id) && !parents.contains(&node.id) {
                warnings.push(PreviewIssue {
                    code: "EMPTY_DIRECTORY".into(),
                    subject: Some(node.id.to_string()),
                });
            }
        }
        if max_depth > 4 {
            warnings.push(PreviewIssue {
                code: "DEEP_DIRECTORY_TREE".into(),
                subject: None,
            });
        }
        issues.sort_by(|a, b| (&a.code, &a.subject).cmp(&(&b.code, &b.subject)));
        warnings.sort_by(|a, b| (&a.code, &a.subject).cmp(&(&b.code, &b.subject)));
        let mut directories: Vec<_> = nodes
            .keys()
            .map(|id| DirectoryCounts {
                directory_id: *id,
                direct_interfaces: counts.get(id).map_or(0, HashSet::len),
            })
            .collect();
        directories.sort_by_key(|d| d.directory_id.to_string());
        PreviewReview {
            structurally_valid: issues.is_empty(),
            issues,
            warnings,
            metrics: PreviewMetrics {
                snapshot_interfaces: expected.len(),
                assigned_interfaces: assigned.len(),
                unclassified_interfaces: unclassified.len(),
                unclassified_ratio: if expected.is_empty() {
                    0.0
                } else {
                    unclassified.len() as f64 / expected.len() as f64
                },
                directory_count: nodes.len(),
                max_depth,
                directories,
                logical_interfaces: Some(expected.len().saturating_sub(reduction)),
            },
        }
    }
}

/// A proposal may generalize whole path segments, but not methods, query strings or path depth.
fn template_covers(template: &str, path: &str) -> bool {
    if !template.starts_with('/')
        || template.len() > 4096
        || template.contains(['?', '#'])
        || template.chars().any(char::is_control)
    {
        return false;
    }
    let t: Vec<_> = template.split('/').collect();
    let p: Vec<_> = path.split('/').collect();
    t.len() == p.len()
        && t.iter().zip(p).all(|(a, b)| {
            if let Some(name) = a.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                !b.is_empty()
                    && !name.is_empty()
                    && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            } else {
                *a == b
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (CatalogSnapshot, DirectoryCandidate) {
        let interface = InterfaceId::new();
        let node = DirectoryId::new();
        (
            CatalogSnapshot {
                project_id: ProjectId::new(),
                project_name: "Fixture".into(),
                interfaces: vec![CatalogInterface {
                    interface_id: interface,
                    method: "GET".into(),
                    path: "/users".into(),
                    recognized_path: None,
                    environments: vec![],
                }],
            },
            DirectoryCandidate {
                merge_groups: vec![],
                nodes: vec![PreviewNode {
                    id: node,
                    parent: None,
                    name: "用户管理".into(),
                    description: "用户相关接口".into(),
                }],
                assignments: vec![PreviewAssignment {
                    interface_id: interface,
                    directory_id: Some(node),
                    reason: "路径为/users".into(),
                }],
            },
        )
    }
    #[test]
    fn merge_groups_keep_coverage_and_reject_unsafe_membership() {
        let (mut s, mut c) = fixture();
        s.interfaces[0].path = "/users/123".into();
        let mut other = s.interfaces[0].clone();
        other.interface_id = InterfaceId::new();
        other.path = "/users/456".into();
        s.interfaces.push(other);
        let mut assignment = c.assignments[0].clone();
        assignment.interface_id = s.interfaces[1].interface_id;
        c.assignments.push(assignment);
        c.merge_groups.push(InterfaceMergeGroup {
            representative_id: s.interfaces[0].interface_id,
            member_ids: s.interfaces.iter().map(|i| i.interface_id).collect(),
            path_template: "/users/{param1}".into(),
            reason: "Only the numeric route parameter differs".into(),
        });
        let reviewer = StructuralDirectoryReviewer;
        let r = reviewer.review(&s, &c);
        assert!(r.structurally_valid);
        assert_eq!(r.metrics.logical_interfaces, Some(1));
        assert_eq!(r.metrics.assigned_interfaces, 2);
        let mut wrong = s.clone();
        wrong.interfaces[1].method = "POST".into();
        assert!(!reviewer.review(&wrong, &c).structurally_valid);
        let mut wrong = c.clone();
        wrong.merge_groups[0].path_template = "/admin/{id}".into();
        assert!(!reviewer.review(&s, &wrong).structurally_valid);
        let mut wrong = c.clone();
        wrong.merge_groups.push(wrong.merge_groups[0].clone());
        assert!(!reviewer.review(&s, &wrong).structurally_valid);
        let mut wrong = c.clone();
        wrong.assignments[1].directory_id = None;
        assert!(!reviewer.review(&s, &wrong).structurally_valid);
        let mut wrong = c.clone();
        wrong.merge_groups[0].member_ids.push(InterfaceId::new());
        assert!(!reviewer.review(&s, &wrong).structurally_valid);
    }
    #[test]
    fn valid_and_explicitly_unclassified_are_distinct_from_missing() {
        let (s, mut c) = fixture();
        let evaluator = StructuralDirectoryReviewer;
        let r = evaluator.review(&s, &c);
        assert!(r.structurally_valid);
        assert_eq!(r.metrics.assigned_interfaces, 1);
        c.assignments[0].directory_id = None;
        let r = evaluator.review(&s, &c);
        assert!(r.structurally_valid);
        assert_eq!(r.metrics.unclassified_ratio, 1.0);
        c.assignments.clear();
        assert!(!evaluator.review(&s, &c).structurally_valid);
    }
    #[test]
    fn rejects_broken_graphs_duplicate_membership_and_unknown_references() {
        let (s, base) = fixture();
        let evaluator = StructuralDirectoryReviewer;
        let mut variants = Vec::new();
        let mut c = base.clone();
        c.nodes[0].parent = Some(c.nodes[0].id);
        variants.push((c, "DIRECTORY_CYCLE"));
        let mut c = base.clone();
        c.nodes[0].parent = Some(DirectoryId::new());
        variants.push((c, "UNKNOWN_PARENT"));
        let mut c = base.clone();
        c.assignments.push(c.assignments[0].clone());
        variants.push((c, "DUPLICATE_ASSIGNMENT"));
        let mut c = base.clone();
        c.assignments[0].interface_id = InterfaceId::new();
        variants.push((c, "UNKNOWN_INTERFACE"));
        let mut c = base.clone();
        c.assignments[0].directory_id = Some(DirectoryId::new());
        variants.push((c, "UNKNOWN_DIRECTORY"));
        let mut c = base.clone();
        c.nodes.push(c.nodes[0].clone());
        variants.push((c, "DUPLICATE_DIRECTORY_ID"));
        let mut c = base.clone();
        c.nodes[0].name = " ".into();
        variants.push((c, "INVALID_DIRECTORY_NAME"));
        let mut c = base.clone();
        c.assignments[0].reason = "".into();
        variants.push((c, "INVALID_ASSIGNMENT_REASON"));
        for (c, code) in variants {
            let r = evaluator.review(&s, &c);
            assert!(!r.structurally_valid);
            assert!(r.issues.iter().any(|i| i.code == code), "expected {code}");
        }
    }
    #[test]
    fn child_before_parent_and_multiple_roots_are_supported() {
        let (s, mut c) = fixture();
        let parent = DirectoryId::new();
        c.nodes[0].parent = Some(parent);
        c.nodes.push(PreviewNode {
            id: parent,
            parent: None,
            name: "业务服务".into(),
            description: "".into(),
        });
        let r = StructuralDirectoryReviewer.review(&s, &c);
        assert!(r.structurally_valid);
        assert_eq!(r.metrics.max_depth, 2);
    }
}

//! Project-wide, reversible decisions, independent of evidence confirmation.
use super::*;
use crate::endpoint::{FieldLocation, FieldPath};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LifecycleTarget {
    Folder {
        id: FolderId,
    },
    Endpoint {
        id: EndpointId,
    },
    Field {
        endpoint: EndpointId,
        location: FieldLocation,
        path: FieldPath,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retirement {
    pub target: LifecycleTarget,
    pub reason: String,
    pub actor: CandidateActor,
    pub at: DateTime<Utc>,
    pub version: u64,
    pub retired: bool,
    /// Frozen subtree: a later move cannot accidentally revive a retired child.
    #[serde(default)]
    pub folders: Vec<FolderId>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentLifecycle {
    pub version: u64,
    pub reconciled_version: u64,
    pub decisions: Vec<Retirement>,
    /// Read-time alias resolution. Observe remains the owner of identities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<(EndpointId, EndpointId)>,
}
impl DocumentLifecycle {
    pub fn is_empty(&self) -> bool {
        self.version == 0
    }
    pub fn pending(&self) -> bool {
        self.version != self.reconciled_version
    }
    pub fn direct_count(&self) -> usize {
        self.decisions.iter().filter(|d| d.retired).count()
    }
    fn same_endpoint(&self, a: EndpointId, b: EndpointId) -> bool {
        // Observe returns aliases resolved to their canonical identity. Two
        // distinct historical aliases must compare equal through that identity.
        let canonical = |id| {
            self.aliases
                .iter()
                .find(|(old, _)| *old == id)
                .map_or(id, |(_, current)| *current)
        };
        canonical(a) == canonical(b)
    }
    pub fn endpoint(&self, id: EndpointId) -> Option<&Retirement> {
        self.decisions.iter().find(|d| d.retired && matches!(d.target, LifecycleTarget::Endpoint { id: old } if self.same_endpoint(old, id)))
    }
    pub fn folder(&self, id: FolderId) -> Option<&Retirement> {
        self.decisions.iter().find(|d| {
            d.retired
                && (matches!(d.target, LifecycleTarget::Folder { id: old } if old == id)
                    || d.folders.contains(&id))
        })
    }
    pub fn field(
        &self,
        endpoint: EndpointId,
        location: FieldLocation,
        path: &FieldPath,
    ) -> Option<&Retirement> {
        self.endpoint(endpoint).or_else(|| self.decisions.iter().find(|d| d.retired && matches!(&d.target,
            LifecycleTarget::Field { endpoint: old, location: at, path: parent } if self.same_endpoint(*old, endpoint) && *at == location && path.0.starts_with(&parent.0))))
    }
    /// Qualified keys are the existing knowledge contract; use location and exact
    /// segment paths at the management boundary (keys alone can be ambiguous).
    pub fn field_key(&self, endpoint: EndpointId, key: &str) -> Option<&Retirement> {
        self.endpoint(endpoint).or_else(|| {
            self.decisions.iter().find(|d| {
                if !d.retired {
                    return false;
                }
                let LifecycleTarget::Field {
                    endpoint: old,
                    location,
                    path,
                } = &d.target
                else {
                    return false;
                };
                if !self.same_endpoint(*old, endpoint) {
                    return false;
                }
                let parent = crate::knowledge::qualified_field_key(*location, path);
                key == parent
                    || key.strip_prefix(&parent).is_some_and(|rest| {
                        rest.starts_with('.') || rest.starts_with("[]") || path.0.is_empty()
                    })
            })
        })
    }
    pub fn source_field(
        &self,
        endpoint: EndpointId,
        location: Option<FieldLocation>,
        path: &str,
    ) -> Option<&Retirement> {
        if self.field_key(endpoint, path).is_some() {
            return self.field_key(endpoint, path);
        }
        if let Some(location) = location {
            let prefix = qualified_field_key(location, &FieldPath::default());
            self.field_key(endpoint, &format!("{prefix}{path}"))
        } else {
            self.field_key(endpoint, path)
        }
    }
    pub fn link_retired(&self, link: &Link) -> bool {
        self.endpoint(link.from).is_some()
            || self.endpoint(link.to).is_some()
            || link.context.source_input.as_ref().is_some_and(|i| {
                self.source_field(i.endpoint, Some(i.location), &i.path)
                    .is_some()
            })
            || link.context.matched_fields.iter().any(|m| {
                self.source_field(link.from, link.context.from_location, &m.from)
                    .is_some()
                    || self
                        .source_field(link.to, link.context.to_location, &m.to)
                        .is_some()
            })
            || link.from_field.as_ref().is_some_and(|key| {
                self.source_field(link.from, link.context.from_location, key)
                    .is_some()
            })
            || link.to_field.as_ref().is_some_and(|key| {
                self.source_field(link.to, link.context.to_location, key)
                    .is_some()
            })
    }
    pub fn source_retired(&self, source: &EvidenceSource) -> bool {
        let Some(endpoint) = source.endpoint else {
            return false;
        };
        if self.endpoint(endpoint).is_some() {
            return true;
        }
        let (Some(location), Some(pointer)) = (source.location, source.pointer.as_deref()) else {
            return false;
        };
        // JSON pointers retain exact segment boundaries; numeric array indices
        // match Items. Ambiguous non-pointer references remain evidence to review.
        let parts: Vec<_> = pointer
            .strip_prefix('/')
            .map(|s| {
                s.split('/')
                    .map(|p| p.replace("~1", "/").replace("~0", "~"))
                    .collect()
            })
            .unwrap_or_default();
        self.decisions.iter().any(|d| d.retired && matches!(&d.target, LifecycleTarget::Field {endpoint: id, location: at, path} if self.same_endpoint(*id,endpoint) && *at==location && (path.0.is_empty() || (pointer.starts_with('/') && path.0.len() <= parts.len() && path.0.iter().zip(&parts).all(|(segment,part)| match segment { crate::endpoint::PathSegment::Key(key) => key==part, crate::endpoint::PathSegment::Items => part.parse::<usize>().is_ok() })))))
    }
    pub fn field_sources_retired(&self, field: &FieldNote) -> bool {
        field.context.evidence.iter().any(|e| {
            self.source_field(
                e.endpoint,
                e.source.as_ref().and_then(|s| s.location),
                &e.field,
            )
            .is_some()
                || e.source.as_ref().is_some_and(|s| self.source_retired(s))
        })
    }
    pub fn retired_source_count(&self, knowledge: &EndpointKnowledge) -> usize {
        knowledge
            .links
            .iter()
            .filter(|l| self.link_retired(l))
            .count()
            + knowledge
                .fields
                .iter()
                .filter(|f| self.field_sources_retired(f))
                .count()
            + knowledge.note.as_ref().map_or(0, |n| {
                n.context
                    .sources
                    .iter()
                    .filter(|s| self.source_retired(s))
                    .count()
            })
    }
    pub fn project_snapshot(&self, view: &mut KnowledgeSnapshot) {
        view.endpoints.retain(|id| self.endpoint(*id).is_none());
        view.folders.retain(|f| self.folder(f.id).is_none());
        view.placements
            .retain(|p| self.endpoint(p.endpoint).is_none() && self.folder(p.folder).is_none());
        view.notes.retain(|n| self.endpoint(n.endpoint).is_none());
        view.fields
            .retain(|n| self.field_key(n.endpoint, &n.path).is_none());
        view.links.retain(|l| !self.link_retired(l));
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleChange {
    pub target: LifecycleTarget,
    pub retired: bool,
    pub reason: String,
    pub expected_version: u64,
}
#[async_trait]
pub trait DocumentLifecycleWriter: Send + Sync {
    async fn change_lifecycle(
        &self,
        project: ProjectId,
        change: LifecycleChange,
        actor: CandidateActor,
    ) -> KnowledgeResult<DocumentLifecycle>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_old_aliases_share_retirement_through_the_canonical_identity() {
        let a = EndpointId::new();
        let b = EndpointId::new();
        let canonical = EndpointId::new();
        let mut state = DocumentLifecycle {
            version: 1,
            reconciled_version: 0,
            aliases: vec![(a, canonical), (b, canonical)],
            decisions: vec![Retirement {
                target: LifecycleTarget::Endpoint { id: a },
                reason: "fixture".into(),
                actor: CandidateActor::Unknown,
                at: DateTime::from_timestamp(0, 0).unwrap(),
                version: 1,
                retired: true,
                folders: vec![],
            }],
        };
        assert!(state.endpoint(b).is_some());
        state.decisions[0].target = LifecycleTarget::Field {
            endpoint: a,
            location: FieldLocation::RequestBody,
            path: FieldPath(vec![crate::endpoint::PathSegment::Key("data".into())]),
        };
        assert!(state.field_key(b, "request_body:data.id").is_some());
        assert!(
            state
                .source_field(b, Some(FieldLocation::RequestBody), "data.id")
                .is_some()
        );
        assert!(state.field_key(b, "query:data.id").is_none());
    }
}

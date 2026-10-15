//! Bounded saved-knowledge search. Callers authorize and resolve endpoint scope first.
use super::*;
use crate::endpoint::FieldLocation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEndpoint {
    pub id: EndpointId,
    pub method: String,
    pub path: String,
    /// Exact keys and descendant prefixes compiled from DocumentLifecycle.
    pub excluded_keys: Vec<String>,
    pub excluded_prefixes: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct KnowledgeSearch {
    pub project: ProjectId,
    /// Already ordered by lifecycle, then canonical endpoint ID.
    pub endpoints: Vec<SearchEndpoint>,
    pub fields: bool,
    pub words: Vec<String>,
    pub offset: usize,
    pub limit: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchField {
    pub key: String,
    pub name: Option<String>,
    pub location: Option<FieldLocation>,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeHit {
    pub endpoint: EndpointId,
    pub name: Option<String>,
    pub purpose: Option<String>,
    pub field: Option<SearchField>,
}
pub struct KnowledgeSearchPage {
    pub items: Vec<KnowledgeHit>,
    pub total: usize,
    /// Adapter-specific content digest; excludes live traffic counters.
    pub revision: String,
}
impl KnowledgeSearch {
    /// Reference implementation for non-database readers and contract tests.
    pub fn select(&self, notes: &[EndpointNote], fields: &[FieldNote]) -> KnowledgeSearchPage {
        let mut items = Vec::new();
        let matches = |text: String| self.words.iter().all(|w| text.to_lowercase().contains(w));
        let preview = |s: &str| s.chars().take(320).collect::<String>();
        for endpoint in &self.endpoints {
            let note = notes.iter().find(|n| n.endpoint == endpoint.id);
            let hit = KnowledgeHit {
                endpoint: endpoint.id,
                name: note.and_then(|n| n.name.clone()),
                purpose: note.and_then(|n| n.purpose.as_deref()).map(preview),
                field: None,
            };
            if self.fields {
                let mut fields: Vec<_> = fields
                    .iter()
                    .filter(|n| n.endpoint == endpoint.id)
                    .collect();
                fields.sort_by(|a, b| a.path.cmp(&b.path));
                for field in fields {
                    if endpoint.excluded_keys.contains(&field.path)
                        || endpoint
                            .excluded_prefixes
                            .iter()
                            .any(|p| field.path.starts_with(p))
                        || !matches(format!(
                            "{} {} {}",
                            field.path,
                            field.context.display_name.as_deref().unwrap_or(""),
                            field.text
                        ))
                    {
                        continue;
                    }
                    items.push(KnowledgeHit {
                        field: Some(SearchField {
                            key: field.path.clone(),
                            name: field.context.display_name.clone(),
                            location: field.context.location,
                            description: preview(&field.text),
                        }),
                        ..hit.clone()
                    });
                }
            } else if matches(format!(
                "{} {} {} {}",
                endpoint.method,
                endpoint.path,
                note.and_then(|n| n.name.as_deref()).unwrap_or(""),
                note.and_then(|n| n.purpose.as_deref()).unwrap_or("")
            )) {
                items.push(hit);
            }
        }
        let revision =
            FrozenRef::of(&serde_json::to_string(&items).expect("search hits serialize")).sha256;
        KnowledgeSearchPage {
            total: items.len(),
            revision,
            items: items
                .into_iter()
                .skip(self.offset)
                .take(self.limit)
                .collect(),
        }
    }
}

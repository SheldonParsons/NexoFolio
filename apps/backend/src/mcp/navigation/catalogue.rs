//! Directory reads use a single metadata/membership snapshot, not N+1 folder queries.
use super::projection::{action, page_in_context, preview};
use super::*;
use nexofolio_common::FolderId;
use nexofolio_contracts::knowledge::{CatalogueNavigation, Folder};
use std::collections::{HashMap, HashSet};
pub(super) fn subtree(folders: &[Folder], root: FolderId, recursive: bool) -> HashSet<FolderId> {
    let mut ids = HashSet::from([root]);
    if recursive {
        loop {
            let before = ids.len();
            for f in folders {
                if f.parent.is_some_and(|p| ids.contains(&p)) {
                    ids.insert(f.id);
                }
            }
            if before == ids.len() {
                break;
            }
        }
    }
    ids
}
pub(super) fn in_environment(s: &EndpointSummary, env: Option<EnvironmentId>) -> bool {
    env.is_none_or(|e| s.environments.iter().any(|u| u.environment_id == e))
}
pub(super) fn selected(
    q: &Query,
    s: &CatalogueNavigation,
    summaries: &[EndpointSummary],
) -> Result<HashSet<EndpointId>, ReadError> {
    if q.unclassified && q.directory_id.is_some() {
        return Err("INVALID_ARGUMENTS".into());
    }
    if q.recursive && q.directory_id.is_none() {
        return Err("DIRECTORY_REQUIRED".into());
    }
    let folders = if let Some(id) = q.directory_id {
        if !s.catalogue.folders.iter().any(|f| f.id == id) {
            return Err("DIRECTORY_NOT_IN_PROJECT".into());
        }
        Some(subtree(&s.catalogue.folders, id, q.recursive))
    } else {
        None
    };
    let placements: HashMap<_, _> = s
        .placements
        .iter()
        .map(|p| (p.endpoint, p.folder))
        .collect();
    Ok(summaries
        .iter()
        .filter(|e| in_environment(e, q.environment_id))
        .filter(|e| {
            if q.unclassified {
                // Effective membership ignores a retired directory, without
                // rewriting the historical placement or hiding an exact read.
                return !placements.get(&e.id).is_some_and(|folder| {
                    q.include_retired || q.lifecycle.folder(*folder).is_none()
                });
            }
            folders
                .as_ref()
                .is_none_or(|ids| placements.get(&e.id).is_some_and(|f| ids.contains(f)))
        })
        .map(|e| e.id)
        .collect())
}
pub(super) fn breadcrumb(folders: &[Folder], id: FolderId) -> Vec<Value> {
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut at = Some(id);
    while let Some(id) = at {
        if !seen.insert(id) {
            break;
        }
        let Some(f) = folders.iter().find(|f| f.id == id) else {
            break;
        };
        path.push(json!({"directory_id":id,"name":f.name}));
        at = f.parent;
    }
    path.reverse();
    path
}
impl Navigation {
    pub(super) async fn browse(
        &self,
        q: &Query,
        summaries: &[EndpointSummary],
    ) -> Result<Value, ReadError> {
        let snapshot = self
            .knowledge
            .catalogue_navigation(q.project_id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        let ids = selected(q, &snapshot, summaries)?;
        let folders = &snapshot.catalogue.folders;
        let placements: Vec<_> = snapshot
            .placements
            .iter()
            .filter(|p| q.include_retired || q.lifecycle.folder(p.folder).is_none())
            .collect();
        let env_ids: HashSet<_> = summaries
            .iter()
            .filter(|s| {
                in_environment(s, q.environment_id) && super::lifecycle::visible(q, s.id, None)
            })
            .map(|s| s.id)
            .collect();
        let section = q.section.as_deref().unwrap_or("directories");
        let items = match section {
            "directories" => {
                if q.unclassified || q.recursive {
                    return Err("INVALID_ARGUMENTS".into());
                }
                folders.iter().filter(|f|f.parent==q.directory_id && (q.include_retired || q.lifecycle.folder(f.id).is_none())).map(|f|{
     let descendants=subtree(folders,f.id,true);
     let direct=placements.iter().filter(|p|p.folder==f.id&&env_ids.contains(&p.endpoint)).count();
     let deep=placements.iter().filter(|p|descendants.contains(&p.folder)&&env_ids.contains(&p.endpoint)).count();
     json!({"directory_id":f.id,"name":f.name,"summary":f.summary.as_deref().map(preview),"summary_preview":true,
      "read_description":action("read_source_evidence",q,json!({"directory_id":f.id,"endpoint_id":Value::Null})),"path":breadcrumb(folders,f.id),"direct_interfaces":direct,"subtree_interfaces":deep,
      "child_directories":folders.iter().filter(|c|c.parent==Some(f.id) && (q.include_retired || q.lifecycle.folder(c.id).is_none())).count(),
      "open":action("browse_directory",q,json!({"directory_id":f.id,"section":"directories"})),
      "interfaces":action("browse_directory",q,json!({"directory_id":f.id,"section":"interfaces"}))})
    }).collect()
            }
            "interfaces" => {
                let notes = self
                    .knowledge
                    .notes(q.project_id)
                    .await
                    .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
                summaries
                    .iter()
                    .filter(|s| ids.contains(&s.id) && super::lifecycle::visible(q, s.id, None))
                    .map(|s| super::search::card(q, s, notes.iter().find(|n| n.endpoint == s.id)))
                    .collect()
            }
            _ => return Err("INVALID_ARGUMENTS".into()),
        };
        let directory=q.directory_id.and_then(|id|folders.iter().find(|f|f.id==id)).map(|f|json!({"directory_id":f.id,"name":f.name,"summary_preview":f.summary.as_deref().map(preview),"full_description":action("read_source_evidence",q,json!({"directory_id":f.id})),"path":breadcrumb(folders,f.id)})).unwrap_or(Value::Null);
        let context = json!({"directory":directory,"directory_count":folders.len()});
        let mut result = page_in_context("browse_directory", q, items, &context)?;
        result["directory"] = directory;
        result["unclassified"] = action(
            "browse_directory",
            q,
            json!({"section":"interfaces","unclassified":true}),
        );
        result["all_interfaces"] = action("search_interfaces", q, json!({"query":""}));
        result["directory_count"] = json!(folders.len());
        result["scope_note"] = json!(
            "Folder metadata is saved project knowledge. Interface counts and membership are filtered by the selected observed environment; zero does not prove no business API exists."
        );
        if folders.is_empty() {
            result["directory_state"] = json!("no_saved_directories");
        }
        Ok(result)
    }
}

use crate::store_error;
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge_contracts::KnowledgeTx;
use sha2::{Digest, Sha256};

pub(super) async fn capture(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    endpoints: Vec<EndpointId>,
) -> KnowledgeResult<KnowledgeSnapshot> {
    let folders = tx
        .folders(project)
        .await
        .map_err(store_error)?
        .into_iter()
        .map(|f| Folder {
            id: f.id,
            author: f.author,
            parent: f.parent,
            name: f.name,
            summary: f.summary,
            position: f.position,
            endpoints: 0,
            endpoints_deep: 0,
        })
        .collect();
    let placements = tx
        .placements(project)
        .await
        .map_err(store_error)?
        .into_iter()
        .map(|p| Placement {
            endpoint: p.endpoint,
            folder: p.folder,
            author: p.author,
            at: p.at,
        })
        .collect();
    let notes = tx.endpoint_notes(project).await.map_err(store_error)?;
    let links = tx.links(project).await.map_err(store_error)?;
    let mut fields = Vec::new();
    for endpoint in &endpoints {
        fields.extend(tx.field_notes(*endpoint).await.map_err(store_error)?);
    }
    Ok(KnowledgeSnapshot {
        endpoints,
        folders,
        placements,
        notes,
        fields,
        links,
    })
}

pub(super) fn hash(snapshot: &KnowledgeSnapshot) -> String {
    let mut canonical = snapshot.clone();
    canonical.endpoints.sort_by_key(|id| id.to_string());
    canonical.folders.sort_by_key(|f| f.id.to_string());
    canonical.placements.sort_by_key(|p| p.endpoint.to_string());
    canonical.notes.sort_by_key(|n| n.endpoint.to_string());
    canonical
        .fields
        .sort_by_key(|n| (n.endpoint.to_string(), n.path.clone()));
    canonical.links.sort_by_key(|l| l.id.to_string());
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical).expect("knowledge snapshot"))
    )
}

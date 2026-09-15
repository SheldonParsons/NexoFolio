use async_trait::async_trait;
use nexofolio_contracts::{Error, ProjectId, Result};
use nexofolio_evidence::{BlobRef, BlobStore};
use sha2::{Digest, Sha256};
use std::{io::Write, path::PathBuf, sync::Arc};
#[derive(Clone)]
pub struct FileBlobStore {
    root: Arc<PathBuf>,
}
fn unavailable() -> Error {
    Error::Unavailable {
        component: "blob_storage",
    }
}
fn location(root: &std::path::Path, project: ProjectId, hash: &str) -> Result<PathBuf> {
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::invalid("invalid content reference"));
    }
    Ok(root.join(project.to_string()).join(&hash[..2]).join(hash))
}
impl FileBlobStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: Arc::new(root),
        }
    }
}
#[async_trait]
impl BlobStore for FileBlobStore {
    async fn put(&self, project: ProjectId, bytes: Vec<u8>, media: &str) -> Result<BlobRef> {
        let hash = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let len = bytes.len() as u64;
        let root = self.root.clone();
        let path = location(&root, project, &hash)?;
        tokio::task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir_all(&*root).map_err(|_| unavailable())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&*root, std::fs::Permissions::from_mode(0o700))
                    .map_err(|_| unavailable())?;
            }
            if path.exists() {
                let existing = std::fs::read(&path).map_err(|_| unavailable())?;
                if Sha256::digest(&existing) != Sha256::digest(&bytes) {
                    return Err(Error::Unavailable {
                        component: "blob_integrity",
                    });
                }
                return Ok(());
            }
            let parent = path.parent().expect("parent");
            std::fs::create_dir_all(parent).map_err(|_| unavailable())?;
            let tmp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .map_err(|_| unavailable())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))
                    .map_err(|_| unavailable())?;
            }
            if file
                .write_all(&bytes)
                .and_then(|_| file.sync_all())
                .and_then(|_| std::fs::rename(&tmp, &path))
                .is_err()
            {
                let _ = std::fs::remove_file(&tmp);
                return Err(unavailable());
            }
            std::fs::File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(|_| unavailable())?;
            Ok(())
        })
        .await
        .map_err(|_| unavailable())??;
        Ok(BlobRef {
            sha256: hash,
            bytes: len,
            media_type: media.into(),
        })
    }
    async fn get(&self, project: ProjectId, hash: &str) -> Result<Vec<u8>> {
        let path = location(&self.root, project, hash)?;
        let expected = hash.to_owned();
        tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(path).map_err(|_| unavailable())?;
            let actual = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            if actual != expected {
                return Err(Error::Unavailable {
                    component: "blob_integrity",
                });
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| unavailable())?
    }
    async fn delete(&self, project: ProjectId, hash: &str) -> Result<()> {
        let path = location(&self.root, project, hash)?;
        tokio::task::spawn_blocking(move || match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(unavailable()),
        })
        .await
        .map_err(|_| unavailable())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn content_survives_adapter_restart_and_corruption_is_explicit() {
        let root = std::env::temp_dir().join(format!("nexo-blob-{}", uuid::Uuid::new_v4()));
        let project = ProjectId::new();
        let store = FileBlobStore::new(root.clone());
        let first = store
            .put(
                project,
                b"source-evidence".to_vec(),
                "application/octet-stream",
            )
            .await
            .unwrap();
        let repeated = store
            .put(
                project,
                b"source-evidence".to_vec(),
                "application/octet-stream",
            )
            .await
            .unwrap();
        assert_eq!(first.sha256, repeated.sha256);
        let restarted = FileBlobStore::new(root.clone());
        assert_eq!(
            restarted.get(project, &first.sha256).await.unwrap(),
            b"source-evidence"
        );
        let file = location(&root, project, &first.sha256).unwrap();
        std::fs::write(file, b"corrupt").unwrap();
        assert!(restarted.get(project, &first.sha256).await.is_err());
        assert!(
            restarted
                .put(
                    project,
                    b"source-evidence".to_vec(),
                    "application/octet-stream"
                )
                .await
                .is_err()
        );
        assert!(restarted.get(project, "../outside").await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn unwritable_storage_is_not_success() {
        let root =
            std::env::temp_dir().join(format!("nexo-not-a-directory-{}", uuid::Uuid::new_v4()));
        std::fs::write(&root, b"file").unwrap();
        let store = FileBlobStore::new(root.clone());
        assert!(
            store
                .put(ProjectId::new(), b"evidence".to_vec(), "application/json")
                .await
                .is_err()
        );
        std::fs::remove_file(root).unwrap();
    }
}

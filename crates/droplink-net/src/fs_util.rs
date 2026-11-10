use droplink_core::path::candidate_names;
use droplink_core::{Error, Result};
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};

/// Create a unique `.part` file in `dir` using atomic `create_new`.
pub async fn create_part(dir: &Path, base_name: &str) -> Result<(PathBuf, File)> {
    tokio::fs::create_dir_all(dir).await?;
    for name in candidate_names(base_name) {
        let part = dir.join(format!("{name}.part"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .await
        {
            Ok(file) => return Ok((part, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Err(Error::Busy)
}

/// Publish a verified `.part` file under a unique final name.
///
/// `hard_link` fails atomically when the target exists, so two concurrent transfers of the same name
/// can never overwrite each other. Filesystems without hard links fall back to an atomic
/// `create_new` claim followed by a rename.
pub async fn publish(part: &Path, dir: &Path, base_name: &str) -> Result<PathBuf> {
    for name in candidate_names(base_name) {
        let final_path = dir.join(&name);
        match tokio::fs::hard_link(part, &final_path).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(part).await;
                return Ok(final_path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                let claim = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&final_path)
                    .await;
                match claim {
                    Ok(file) => {
                        drop(file);
                        tokio::fs::rename(part, &final_path).await?;
                        return Ok(final_path);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(Error::Io(e)),
                }
            }
        }
    }
    Err(Error::Busy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[tokio::test]
    async fn create_unique_concurrent_10() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        let mut handles = Vec::new();
        for _ in 0..10 {
            let dir = dir.clone();
            handles.push(tokio::spawn(async move {
                let (path, mut file) = create_part(&dir, "same.txt").await.unwrap();
                use tokio::io::AsyncWriteExt;
                file.write_all(b"x").await.unwrap();
                path
            }));
        }
        let mut seen = HashSet::new();
        for h in handles {
            seen.insert(h.await.unwrap());
        }
        assert_eq!(seen.len(), 10);
    }

    #[tokio::test]
    async fn publish_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let mut paths = Vec::new();
        for i in 0..3 {
            let (part, mut file) = create_part(base, "photo.jpg").await.unwrap();
            use tokio::io::AsyncWriteExt;
            file.write_all(format!("content {i}").as_bytes())
                .await
                .unwrap();
            drop(file);
            paths.push(publish(&part, base, "photo.jpg").await.unwrap());
        }
        assert_eq!(paths.len(), 3);
        let unique: HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), 3);
        assert!(base.join("photo.jpg").exists());
        assert!(base.join("photo (1).jpg").exists());
        assert!(base.join("photo (2).jpg").exists());
    }

    #[tokio::test]
    async fn publish_leaves_no_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let (part, file) = create_part(base, "a.bin").await.unwrap();
        drop(file);
        let published = publish(&part, base, "a.bin").await.unwrap();
        assert!(!part.exists());
        assert!(published.exists());
    }
}

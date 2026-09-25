//! Durable synchronization files are not source files or derived caches.
//! Lock files must remain in place: unlinking one while a process holds its
//! inode permits a second writer to acquire a different lock.
use crate::digest::digest_bytes;
use anyhow::{bail, Result};
use std::fs;
use std::path::PathBuf;

pub fn lock_path(kind: &str, identity: &str) -> Result<PathBuf> {
    let cache = crate::cache::directory()?;
    let parent = cache
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid cache location"))?;
    let directory = parent.join("wt-locks");
    fs::create_dir_all(&directory)?;
    if fs::symlink_metadata(&directory)?.file_type().is_symlink() {
        bail!("coordination directory must not be a symlink")
    }
    let digest = digest_bytes(identity.as_bytes());
    Ok(directory.join(format!("{kind}-{}.lock", &digest[7..])))
}

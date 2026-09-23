use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub(crate) const MAX_PACKAGE_FILE_BYTES: usize = 4 * 1024 * 1024;

pub fn digest_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

pub fn digest_text(text: &str) -> String {
    digest_bytes(text.as_bytes())
}

/// WT-PACKAGE-1 framing.  Paths are normalized, sorted, unique, and all
/// lengths are unsigned big-endian values so the identity is portable.
pub fn digest_package(root: &Path, references: &[&str], manifest: &[u8]) -> Result<String> {
    digest_package_with_contents(root, references, manifest, &BTreeMap::new())
}

pub(crate) fn digest_package_with_contents(
    root: &Path,
    references: &[&str],
    manifest: &[u8],
    contents: &BTreeMap<String, Vec<u8>>,
) -> Result<String> {
    let mut paths = references
        .iter()
        .map(|path| normalize_reference(path))
        .collect::<Result<Vec<_>>>()?;
    paths.push("rule.json".to_owned());
    paths.sort();
    paths.dedup();

    let mut hasher = Sha256::new();
    write_part(&mut hasher, b"WT-PACKAGE-1");
    for path in paths {
        let bytes = if path == "rule.json" {
            manifest.to_owned()
        } else if let Some(bytes) = contents.get(&path) {
            bytes.clone()
        } else {
            read_package_file(root, &path, MAX_PACKAGE_FILE_BYTES)?
        };
        write_part(&mut hasher, path.as_bytes());
        write_part(&mut hasher, &bytes);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub(crate) fn normalize_reference(reference: &str) -> Result<String> {
    if reference.is_empty() || reference.contains('\\') || Path::new(reference).is_absolute() {
        bail!("unsafe package reference {reference:?}");
    }
    let mut components = Vec::new();
    for component in Path::new(reference).components() {
        match component {
            Component::Normal(value) => components.push(
                value
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("package reference is not UTF-8"))?,
            ),
            Component::CurDir => {}
            _ => bail!("unsafe package reference {reference:?}"),
        }
    }
    if components.is_empty() {
        bail!("unsafe package reference {reference:?}");
    }
    Ok(components.join("/"))
}

pub(crate) fn package_path(root: &Path, reference: &str) -> Result<(String, PathBuf)> {
    let reference = normalize_reference(reference)?;
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("unable to canonicalize package root {}", root.display()))?;
    let path = root.join(&reference);
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("unable to resolve package reference {reference:?}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        bail!("package reference escapes its package");
    }
    Ok((reference, path))
}

pub(crate) fn read_package_file(root: &Path, reference: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let (reference, path) = package_path(root, reference)?;
    let before = fs::metadata(&path)
        .with_context(|| format!("unable to stat package reference {reference:?}"))?;
    if !before.is_file() {
        bail!("package reference {reference:?} is not a regular file");
    }
    let mut file = File::open(&path)
        .with_context(|| format!("unable to read package reference {reference:?}"))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("unable to read package reference {reference:?}"))?;
    if bytes.len() > max_bytes {
        bail!("package reference {reference:?} exceeds {max_bytes} bytes");
    }
    let after = fs::metadata(&path)?;
    if before.len() != after.len() {
        bail!("package reference {reference:?} changed while being read");
    }
    Ok(bytes)
}

fn write_part(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

pub fn semantic_digest(domain: &str, parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    write_part(&mut hasher, domain.as_bytes());
    for part in parts {
        write_part(&mut hasher, part);
    }
    format!("sha256:{:x}", hasher.finalize())
}

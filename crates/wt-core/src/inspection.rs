//! Best-effort, read-only local VCS evidence for an inspected stale review.
//! Object contents are accepted only after checking the recorded SHA-256.
use crate::digest::digest_bytes;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

const MAX_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIFF_SOURCE_BYTES: u64 = 1024 * 1024;
const MAX_DIFF_BYTES: usize = 256 * 1024;

pub fn previous_diff(
    root: &Path,
    previous: &Value,
    current: Option<&Value>,
) -> Result<Option<String>> {
    let Some(current) = current else {
        return Ok(None);
    };
    let Some(path) = previous["record"]["path"].as_str() else {
        return Ok(None);
    };
    if current["path"] != path {
        return Ok(None);
    }
    let Some(old_digest) = previous["record"]["file_digest"].as_str() else {
        return Ok(None);
    };
    if current["file_digest"] == old_digest {
        return Ok(None);
    }
    let source = root.join(path);
    let mut component = root.to_path_buf();
    for segment in path.split('/') {
        component.push(segment);
        if fs::symlink_metadata(&component)?.file_type().is_symlink() {
            bail!("source became a symlink while inspecting {path}");
        }
    }
    let Some(new) = bounded_file(&source)? else {
        return Ok(None);
    };
    if Some(digest_bytes(&new).as_str()) != current["file_digest"].as_str() {
        bail!("source changed while inspecting {path}; retry");
    }
    if new.len() as u64 > MAX_DIFF_SOURCE_BYTES {
        return Ok(None);
    }
    let Some(git_root) = git(root, &["rev-parse", "--show-toplevel"])? else {
        return Ok(None);
    };
    let git_root = String::from_utf8(git_root)?.trim().to_owned();
    let git_root = Path::new(&git_root).canonicalize()?;
    let relative = source
        .strip_prefix(&git_root)
        .ok()
        .and_then(|path| path.to_str())
        .map(|path| path.replace('\\', "/"));
    let Some(relative) = relative else {
        return Ok(None);
    };
    let Some(revisions) = git(
        &git_root,
        &["rev-list", "-n", "32", "HEAD", "--", &relative],
    )?
    else {
        return Ok(None);
    };
    for revision in String::from_utf8(revisions)?.lines() {
        if revision.len() != 40 && revision.len() != 64 {
            continue;
        }
        let Some(object) = git(&git_root, &["rev-parse", &format!("{revision}:{relative}")])?
        else {
            continue;
        };
        let object = String::from_utf8(object)?.trim().to_owned();
        let Some(size) = git(&git_root, &["cat-file", "-s", &object])? else {
            continue;
        };
        if size.len() > 24
            || size
                .iter()
                .any(|byte| !byte.is_ascii_digit() && *byte != b'\n')
        {
            continue;
        }
        if String::from_utf8(size)?
            .trim()
            .parse::<u64>()
            .unwrap_or(u64::MAX)
            > MAX_DIFF_SOURCE_BYTES
        {
            continue;
        }
        let Some(old) = git(&git_root, &["cat-file", "blob", &object])? else {
            continue;
        };
        if old.len() as u64 > MAX_DIFF_SOURCE_BYTES || digest_bytes(&old) != old_digest {
            continue;
        }
        return diff(&old, &new);
    }
    Ok(None)
}

fn bounded_file(path: &Path) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file: File = options.open(path)?;
    if !file.metadata()?.is_file() {
        bail!("inspected source must be a regular file");
    }
    file.take(MAX_EVIDENCE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_EVIDENCE_BYTES {
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn git(root: &Path, args: &[&str]) -> Result<Option<Vec<u8>>> {
    let output = match Command::new("git").arg("-C").arg(root).args(args).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(output.status.success().then_some(output.stdout))
}

fn diff(old: &[u8], new: &[u8]) -> Result<Option<String>> {
    let mut before = tempfile::NamedTempFile::new()?;
    let mut after = tempfile::NamedTempFile::new()?;
    before.write_all(old)?;
    after.write_all(new)?;
    let output = Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--color=never",
            "--",
        ])
        .arg(before.path())
        .arg(after.path())
        .output()
        .context("unable to diff verified source")?;
    if output.status.code() != Some(1) {
        return Ok(None);
    }
    if output.stdout.len() > MAX_DIFF_BYTES {
        return Ok(None);
    }
    let text = String::from_utf8(output.stdout)?;
    let hunks = text.find("@@ ").map(|index| &text[index..]);
    Ok(hunks.map(str::to_owned))
}

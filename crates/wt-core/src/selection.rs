use crate::config::EffectiveConfig;
use crate::digest::digest_bytes;
use crate::rule::RulePackage;
use anyhow::{anyhow, bail, Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const BINARY_PROBE_BYTES: usize = 8 * 1024;
const MAX_SOURCE_FILES: usize = 10_000;
const MAX_RETAINED_SOURCE_BYTES: usize = 100 * 1024 * 1024;

#[derive(Clone)]
pub struct SelectedFile {
    pub path: String,
    pub text: String,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CoverageItem {
    pub path: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

pub struct Selection {
    /// Materialized source files. Streamed selections intentionally leave this empty.
    #[allow(dead_code)] // Used by the materialized selector's conformance tests.
    pub files: Vec<SelectedFile>,
    pub repository_files: Vec<SelectedFile>,
    pub coverage: Vec<CoverageItem>,
    pub gaps: Vec<CoverageItem>,
    pub partial: bool,
    pub changed_base: Option<String>,
    #[allow(dead_code)]
    pub source_reads: usize,
    #[allow(dead_code)]
    pub bytes_hashed: usize,
    #[allow(dead_code)]
    pub checked_files: usize,
}

struct GitInfo {
    tracked: HashSet<String>,
    changed: Option<HashSet<String>>,
    changed_base: Option<String>,
    repo_root: Option<PathBuf>,
    is_git: bool,
}

struct Candidate {
    path: PathBuf,
    kind: CandidateKind,
    ignored: bool,
}

#[derive(Clone, Copy)]
enum CandidateKind {
    Regular,
    Symlink,
    Other,
    Boundary,
}

struct ScopeFilter {
    file_rules: Vec<ScopeRule>,
    repository_rules: Vec<ScopeRule>,
}

struct ScopeRule {
    include: GlobSet,
    exclude: GlobSet,
}

enum ReadOutcome {
    Bytes(Vec<u8>),
    Binary {
        digest: Option<String>,
        bytes_read: usize,
    },
    Oversized,
}

pub fn select(
    root: &Path,
    config: &EffectiveConfig,
    options: &serde_json::Value,
) -> Result<Selection> {
    let mut no_callback = |_file: &SelectedFile| Ok(());
    select_inner(root, config, options, None, false, &mut no_callback)
}

/// Select only files applicable to the supplied rules before reading source bytes.
#[allow(dead_code)]
pub fn select_scoped(
    root: &Path,
    config: &EffectiveConfig,
    options: &serde_json::Value,
    packages: &[&RulePackage],
) -> Result<Selection> {
    let filter = ScopeFilter::new(root, packages)?;
    let mut no_callback = |_file: &SelectedFile| Ok(());
    select_inner(root, config, options, Some(filter), false, &mut no_callback)
}

/// Enumerate and hash selected sources once, delivering file-rule inputs without retaining them.
#[allow(dead_code)]
pub fn select_streamed<F>(
    root: &Path,
    config: &EffectiveConfig,
    options: &serde_json::Value,
    packages: &[&RulePackage],
    on_file: F,
) -> Result<Selection>
where
    F: FnMut(&SelectedFile) -> Result<()>,
{
    let filter = ScopeFilter::new(root, packages)?;
    let mut on_file = on_file;
    select_inner(root, config, options, Some(filter), true, &mut on_file)
}

fn select_inner<F>(
    root: &Path,
    config: &EffectiveConfig,
    options: &serde_json::Value,
    scope: Option<ScopeFilter>,
    streamed: bool,
    on_file: &mut F,
) -> Result<Selection>
where
    F: FnMut(&SelectedFile) -> Result<()>,
{
    let include_ignored = options
        .get("include_ignored")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let no_host_ignores = options
        .get("no_host_ignores")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || !config.scan.honor_git_local_excludes;
    let changed = options
        .get("changed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let paths = option_paths(options)?;
    if changed && !paths.is_empty() {
        bail!("--changed cannot be combined with positional paths")
    }
    if options.get("base").is_some() && !changed {
        bail!("--base requires --changed")
    }

    let git = git_info(
        root,
        changed,
        options.get("base").and_then(|value| value.as_str()),
        no_host_ignores,
    )?;
    if changed && !git.is_git {
        bail!("--changed requires a Git worktree")
    }
    let exclusions = compile_exclusions(config)?;
    let requested = requested_paths(root, &paths)?;
    let candidates = enumerate_files(
        root,
        git.repo_root.as_deref(),
        config.scan.respect_gitignore,
        config.scan.include_hidden,
        no_host_ignores,
    )?;
    let changed_set = git.changed.as_ref();
    let mut coverage = Vec::new();
    let mut gaps = Vec::new();
    let mut files = Vec::new();
    let mut repository_files = Vec::new();
    let mut seen = HashSet::new();
    let max_bytes = config.scan.max_file_bytes as usize;
    let mut source_file_count: usize = 0;
    let mut retained_source_bytes: usize = 0;
    let mut source_reads = 0;
    let mut bytes_hashed = 0;
    let mut checked_files = 0;

    for candidate in candidates {
        let relative = match relative_path(root, &candidate.path) {
            Ok(relative) => relative,
            Err(_) => {
                gaps.push(item(
                    candidate.path.display().to_string(),
                    "gap",
                    None,
                    Some("non_unicode_path"),
                    Some("working_tree"),
                ));
                continue;
            }
        };
        if !requested.is_empty()
            && !requested.iter().any(|path| {
                path_selected(path, &candidate.path)
                    || (matches!(candidate.kind, CandidateKind::Boundary)
                        && path.starts_with(&candidate.path))
            })
        {
            continue;
        }
        let (file_selected, repository_selected) = scope.as_ref().map_or((true, true), |scope| {
            (
                scope.matches_file(&relative),
                scope.matches_repository(&relative),
            )
        });
        if let Some(_scope) = &scope {
            if !matches!(candidate.kind, CandidateKind::Boundary)
                && !file_selected
                && !repository_selected
            {
                continue;
            }
        }
        seen.insert(relative.clone());

        if matches!(candidate.kind, CandidateKind::Boundary) {
            coverage.push(item(
                relative,
                "excluded",
                None,
                Some("nested_repository"),
                Some("root_boundary"),
            ));
            continue;
        }
        let tracked = git.tracked.contains(&relative);
        if candidate.ignored && !tracked && !include_ignored {
            coverage.push(item(
                relative,
                "ignored",
                None,
                Some("gitignore"),
                Some("gitignore"),
            ));
            continue;
        }
        if exclusions.is_match(&relative) {
            let reason = config
                .scan
                .exclude
                .iter()
                .find(|exclude| exclusion_matches(&exclude.glob, &relative))
                .map(|exclude| exclude.reason.clone())
                .unwrap_or_else(|| "configured exclusion".to_owned());
            coverage.push(item(
                relative,
                "excluded",
                None,
                Some(&reason),
                Some("config.scan.exclude"),
            ));
            continue;
        }
        if matches!(candidate.kind, CandidateKind::Symlink) {
            gaps.push(item(
                relative,
                "gap",
                None,
                Some("symlink_boundary"),
                Some("working_tree"),
            ));
            continue;
        }
        if matches!(candidate.kind, CandidateKind::Other) {
            gaps.push(item(
                relative,
                "gap",
                None,
                Some("unsupported_file_type"),
                Some("working_tree"),
            ));
            continue;
        }

        let changed_file = !changed || changed_set.is_some_and(|set| set.contains(&relative));
        if changed && !repository_selected && !changed_file {
            coverage.push(item(
                relative,
                "unchanged_dependency",
                None,
                Some("changed_selection"),
                Some("changed_selection"),
            ));
            continue;
        }
        let retain_repository = !streamed || repository_selected;
        let file_callback_expected = streamed && file_selected && (!changed || changed_file);
        if retain_repository && source_file_count >= MAX_SOURCE_FILES && !file_callback_expected {
            gaps.push(item(
                relative,
                "gap",
                None,
                Some("source_file_limit"),
                Some("runtime_limit"),
            ));
            continue;
        }
        match read_stable(&candidate.path, max_bytes) {
            Ok(ReadOutcome::Binary { digest, bytes_read }) => {
                source_reads += 1;
                if digest.is_some() {
                    bytes_hashed += bytes_read;
                }
                coverage.push(item(
                    relative,
                    "binary",
                    digest,
                    Some("nul_byte"),
                    Some("working_tree"),
                ));
            }
            Ok(ReadOutcome::Oversized) => {
                source_reads += 1;
                gaps.push(item(
                    relative,
                    "gap",
                    None,
                    Some("oversized"),
                    Some("working_tree"),
                ));
            }
            Ok(ReadOutcome::Bytes(bytes)) => {
                source_reads += 1;
                bytes_hashed += bytes.len();
                let digest = digest_bytes(&bytes);
                let text = match String::from_utf8(bytes) {
                    Ok(value) => value,
                    Err(_) => {
                        gaps.push(item(
                            relative,
                            "gap",
                            None,
                            Some("invalid_utf8"),
                            Some("working_tree"),
                        ));
                        continue;
                    }
                };
                let file = SelectedFile {
                    path: relative.clone(),
                    text,
                    digest: digest.clone(),
                };
                let file_selected_for_callback =
                    streamed && file_selected && (!changed || changed_file);
                if file_selected_for_callback {
                    on_file(&file)?;
                }
                if changed_file {
                    checked_files += 1;
                }
                let exceeds_file_limit = retain_repository && source_file_count >= MAX_SOURCE_FILES;
                let exceeds_byte_limit = retain_repository
                    && retained_source_bytes.saturating_add(file.text.len())
                        > MAX_RETAINED_SOURCE_BYTES;
                let exceeds_repository_budget = exceeds_file_limit || exceeds_byte_limit;
                if exceeds_repository_budget {
                    gaps.push(item(
                        relative,
                        "gap",
                        None,
                        Some(if exceeds_file_limit {
                            "source_file_limit"
                        } else {
                            "source_snapshot_limit"
                        }),
                        Some("runtime_limit"),
                    ));
                    continue;
                }
                if retain_repository {
                    source_file_count += 1;
                    retained_source_bytes += file.text.len();
                    if streamed {
                        repository_files.push(file);
                    } else {
                        repository_files.push(file.clone());
                        if changed_file {
                            files.push(file);
                        }
                    }
                }
                if !streamed {
                    coverage.push(if changed_file {
                        item(
                            relative,
                            "checked",
                            Some(digest),
                            None,
                            Some("working_tree"),
                        )
                    } else {
                        item(
                            relative,
                            "unchanged_dependency",
                            Some(digest),
                            Some("changed_selection"),
                            Some("working_tree"),
                        )
                    });
                } else if changed_file {
                    coverage.push(item(
                        relative,
                        "checked",
                        Some(digest),
                        None,
                        Some("working_tree"),
                    ));
                } else {
                    coverage.push(item(
                        relative,
                        "unchanged_dependency",
                        Some(digest),
                        Some("changed_selection"),
                        Some("working_tree"),
                    ));
                }
            }
            Err(error) => gaps.push(item(
                relative,
                "gap",
                None,
                Some(&error.to_string()),
                Some("working_tree"),
            )),
        }
    }
    if let Some(changed_set) = changed_set {
        for path in changed_set {
            if !seen.contains(path) {
                if scope.as_ref().is_some_and(|filter| {
                    !filter.matches_file(path) && !filter.matches_repository(path)
                }) || exclusions.is_match(path)
                {
                    continue;
                }
                coverage.push(item(
                    path.clone(),
                    "deleted",
                    None,
                    Some("changed_base_difference"),
                    Some("git_diff"),
                ));
            }
        }
    }
    coverage.sort_by(|left, right| left.path.cmp(&right.path));
    gaps.sort_by(|left, right| left.path.cmp(&right.path));
    files.sort_by(|left, right| left.path.cmp(&right.path));
    repository_files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(Selection {
        files,
        repository_files,
        coverage,
        gaps,
        partial: changed || !paths.is_empty(),
        changed_base: if changed { git.changed_base } else { None },
        source_reads,
        bytes_hashed,
        checked_files,
    })
}

fn item(
    path: String,
    status: &str,
    digest: Option<String>,
    reason: Option<&str>,
    origin: Option<&str>,
) -> CoverageItem {
    CoverageItem {
        path,
        status: status.to_owned(),
        digest,
        reason: reason.map(str::to_owned),
        origin: origin.map(str::to_owned),
    }
}

fn option_paths(options: &serde_json::Value) -> Result<Vec<String>> {
    let Some(value) = options.get("paths") else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| anyhow!("paths must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("paths must be strings"))
        })
        .collect()
}

fn requested_paths(root: &Path, paths: &[String]) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for value in paths {
        let path = PathBuf::from(value);
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        let canonical = path
            .canonicalize()
            .with_context(|| format!("unable to resolve path {value}"))?;
        if !canonical.starts_with(root) {
            bail!("path {value:?} is outside the selected root")
        }
        let is_symlink = fs::symlink_metadata(&path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        result.push(if is_symlink { path } else { canonical });
    }
    Ok(result)
}

fn path_selected(requested: &Path, candidate: &Path) -> bool {
    candidate == requested || (requested.is_dir() && candidate.starts_with(requested))
}

fn enumerate_files(
    root: &Path,
    repo_root: Option<&Path>,
    respect_gitignore: bool,
    include_hidden: bool,
    no_host_ignores: bool,
) -> Result<Vec<Candidate>> {
    let mut matchers = Vec::new();
    if !no_host_ignores {
        let (global, error) = GitignoreBuilder::new(root).build_global();
        if let Some(error) = error {
            bail!("unable to read global Git excludes: {error}")
        }
        matchers.push(global);
        if let Some(repo_root) = repo_root {
            if let Some(path) = git_output(
                repo_root,
                &["rev-parse", "--git-path", "info/exclude"],
                true,
            )? {
                let path = PathBuf::from(path.trim());
                let path = if path.is_absolute() {
                    path
                } else {
                    repo_root.join(path)
                };
                if path.is_file() {
                    matchers.push(build_gitignore(&path, repo_root)?);
                }
            }
        }
    }
    if respect_gitignore {
        let base = repo_root
            .filter(|path| root.starts_with(path))
            .unwrap_or(root);
        let mut ancestors = Vec::new();
        let mut current = root;
        loop {
            ancestors.push(current.to_path_buf());
            if current == base {
                break;
            }
            current = current
                .parent()
                .ok_or_else(|| anyhow!("selected root is not under repository root"))?;
        }
        ancestors.reverse();
        for directory in ancestors {
            let path = directory.join(".gitignore");
            if path.is_file() {
                matchers.push(build_gitignore(&path, &directory)?);
            }
        }
    }
    let mut candidates = Vec::new();
    walk_directory(
        root,
        include_hidden,
        respect_gitignore,
        &matchers,
        false,
        &mut candidates,
    )?;
    Ok(candidates)
}

fn walk_directory(
    directory: &Path,
    include_hidden: bool,
    respect_gitignore: bool,
    matchers: &[Gitignore],
    ancestor_ignored: bool,
    candidates: &mut Vec<Candidate>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("unable to read {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let file_type = entry.file_type()?;
        if name == ".git" || name == ".wt" {
            continue;
        }
        if !include_hidden && name.starts_with('.') {
            continue;
        }
        if file_type.is_dir() {
            if path.join(".git").exists() {
                candidates.push(Candidate {
                    ignored: ancestor_ignored || matches_ignore(&path, true, matchers),
                    path,
                    kind: CandidateKind::Boundary,
                });
                continue;
            }
            let mut child_matchers = matchers.to_vec();
            if respect_gitignore {
                let ignore_file = path.join(".gitignore");
                if ignore_file.is_file() {
                    child_matchers.push(build_gitignore(&ignore_file, &path)?);
                }
            }
            walk_directory(
                &path,
                include_hidden,
                respect_gitignore,
                &child_matchers,
                ancestor_ignored || matches_ignore(&path, true, matchers),
                candidates,
            )?;
            continue;
        }
        let kind = if file_type.is_symlink() {
            CandidateKind::Symlink
        } else if file_type.is_file() {
            CandidateKind::Regular
        } else {
            CandidateKind::Other
        };
        candidates.push(Candidate {
            ignored: ancestor_ignored || matches_ignore(&path, false, matchers),
            path,
            kind,
        });
    }
    Ok(())
}

fn matches_ignore(path: &Path, is_dir: bool, matchers: &[Gitignore]) -> bool {
    let mut ignored = false;
    for matcher in matchers {
        let matched = matcher.matched_path_or_any_parents(path, is_dir);
        if matched.is_ignore() {
            ignored = true;
        } else if matched.is_whitelist() {
            ignored = false;
        }
    }
    ignored
}

fn build_gitignore(path: &Path, root: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);
    if let Some(error) = builder.add(path) {
        bail!("unable to read {}: {error}", path.display())
    }
    Ok(builder.build()?)
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let value = path
        .strip_prefix(root)
        .map_err(|_| anyhow!("path is outside root"))?
        .to_str()
        .ok_or_else(|| anyhow!("non-Unicode path is an analysis gap"))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    if value.is_empty() {
        Ok(".".to_owned())
    } else {
        Ok(value)
    }
}

fn compile_exclusions(config: &EffectiveConfig) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for item in &config.scan.exclude {
        builder.add(
            GlobBuilder::new(&item.glob)
                .literal_separator(true)
                .build()?,
        );
    }
    Ok(builder.build()?)
}

fn exclusion_matches(pattern: &str, path: &str) -> bool {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .is_ok_and(|glob| glob.compile_matcher().is_match(path))
}

fn read_stable(path: &Path, max_bytes: usize) -> Result<ReadOutcome> {
    let metadata_before = fs::symlink_metadata(path)?;
    if !metadata_before.file_type().is_file() {
        bail!("unstable_file")
    }
    let modified_before = metadata_before.modified().ok();
    let mut file = File::open(path)?;
    let probe_limit = max_bytes.saturating_add(1).min(BINARY_PROBE_BYTES);
    let mut bytes = Vec::with_capacity(metadata_before.len().min(max_bytes as u64) as usize);
    file.by_ref()
        .take(probe_limit as u64)
        .read_to_end(&mut bytes)?;
    let initial_nul = bytes.contains(&0);
    if metadata_before.len() > max_bytes as u64 {
        let metadata_after = fs::symlink_metadata(path)?;
        ensure_stable(&metadata_before, modified_before, &metadata_after)?;
        return Ok(if initial_nul {
            ReadOutcome::Binary {
                digest: None,
                bytes_read: bytes.len(),
            }
        } else {
            ReadOutcome::Oversized
        });
    }
    if bytes.len() <= max_bytes {
        let remaining = max_bytes + 1 - bytes.len();
        file.take(remaining as u64).read_to_end(&mut bytes)?;
    }
    let metadata_after = fs::symlink_metadata(path)?;
    ensure_stable(&metadata_before, modified_before, &metadata_after)?;
    if bytes.len() > max_bytes {
        return Ok(if initial_nul {
            ReadOutcome::Binary {
                digest: None,
                bytes_read: bytes.len(),
            }
        } else {
            ReadOutcome::Oversized
        });
    }
    Ok(if initial_nul || bytes.contains(&0) {
        ReadOutcome::Binary {
            digest: Some(digest_bytes(&bytes)),
            bytes_read: bytes.len(),
        }
    } else {
        ReadOutcome::Bytes(bytes)
    })
}

fn ensure_stable(
    before: &fs::Metadata,
    modified_before: Option<std::time::SystemTime>,
    after: &fs::Metadata,
) -> Result<()> {
    if after.file_type().is_symlink()
        || before.len() != after.len()
        || modified_before != after.modified().ok()
    {
        bail!("unstable_file")
    }
    Ok(())
}

fn git_info(
    root: &Path,
    changed: bool,
    base: Option<&str>,
    no_host_ignores: bool,
) -> Result<GitInfo> {
    let repo_root = git_output(root, &["rev-parse", "--show-toplevel"], no_host_ignores)?
        .map(|value| PathBuf::from(value.trim()).canonicalize())
        .transpose()?;
    let is_git = repo_root.is_some();
    let tracked = if let Some(repo_root) = repo_root.as_deref() {
        match git_output(repo_root, &["ls-files", "-z"], no_host_ignores)? {
            Some(value) => paths_relative_to_root(root, repo_root, &value)?,
            None => HashSet::new(),
        }
    } else {
        HashSet::new()
    };
    let resolved_base = if changed {
        let repo_root = repo_root
            .as_deref()
            .ok_or_else(|| anyhow!("--changed requires a Git worktree"))?;
        let revision = base.unwrap_or("HEAD");
        let revision_arg = format!("{revision}^{{commit}}");
        Some(
            git_output(
                repo_root,
                &["rev-parse", "--verify", "--end-of-options", &revision_arg],
                no_host_ignores,
            )?
            .ok_or_else(|| anyhow!("unable to resolve Git base {revision}"))?
            .trim()
            .to_owned(),
        )
    } else {
        None
    };
    let changed_set = if changed {
        let repo_root = repo_root
            .as_deref()
            .ok_or_else(|| anyhow!("--changed requires a Git worktree"))?;
        let revision = base.unwrap_or("HEAD");
        let resolved = resolved_base.as_deref().unwrap();
        let names = git_output(
            repo_root,
            &[
                "diff",
                "--name-only",
                "-z",
                "--no-ext-diff",
                "--no-textconv",
                "--end-of-options",
                resolved.trim(),
                "--",
            ],
            no_host_ignores,
        )?
        .ok_or_else(|| anyhow!("unable to read Git changes from {revision}"))?;
        let untracked = git_output(repo_root, &["ls-files", "--others", "-z"], no_host_ignores)?
            .unwrap_or_default();
        let names = format!("{names}{untracked}");
        Some(paths_relative_to_root(root, repo_root, &names)?)
    } else {
        None
    };
    Ok(GitInfo {
        tracked,
        changed: changed_set,
        changed_base: resolved_base,
        repo_root,
        is_git,
    })
}

fn paths_relative_to_root(root: &Path, repo_root: &Path, value: &str) -> Result<HashSet<String>> {
    let prefix = root
        .strip_prefix(repo_root)
        .map_err(|_| anyhow!("selected root is outside Git worktree"))?;
    let mut result = HashSet::new();
    for path in value.split('\0').filter(|value| !value.is_empty()) {
        let path = repo_root.join(path);
        if path.starts_with(root) {
            result.insert(relative_path(root, &path)?);
        } else if prefix.as_os_str().is_empty() {
            result.insert(relative_path(repo_root, &path)?);
        }
    }
    Ok(result)
}

fn git_output(root: &Path, args: &[&str], no_host_ignores: bool) -> Result<Option<String>> {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    if no_host_ignores {
        command
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
    }
    let output = command.output().context("unable to invoke git")?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8(output.stdout)?))
}

impl ScopeFilter {
    #[allow(dead_code)]
    fn new(root: &Path, packages: &[&RulePackage]) -> Result<Self> {
        let mut file_rules = Vec::new();
        let mut repository_rules = Vec::new();
        for package in packages {
            if package.manifest.scope.require_files.iter().any(|required| {
                let path = root.join(required);
                !path.is_file()
                    || path
                        .canonicalize()
                        .map_or(true, |path| !path.starts_with(root))
            }) {
                continue;
            }
            let mut includes = GlobSetBuilder::new();
            for pattern in &package.manifest.scope.include {
                includes.add(GlobBuilder::new(pattern).literal_separator(true).build()?);
            }
            let mut excludes = GlobSetBuilder::new();
            for pattern in &package.manifest.scope.exclude {
                excludes.add(
                    GlobBuilder::new(&pattern.glob)
                        .literal_separator(true)
                        .build()?,
                );
            }
            let rule = ScopeRule {
                include: includes.build()?,
                exclude: excludes.build()?,
            };
            if package.manifest.execution == "repository" {
                repository_rules.push(rule);
            } else {
                file_rules.push(rule);
            }
        }
        Ok(Self {
            file_rules,
            repository_rules,
        })
    }

    fn matches_file(&self, path: &str) -> bool {
        self.file_rules
            .iter()
            .any(|rule| rule.include.is_match(path) && !rule.exclude.is_match(path))
    }

    fn matches_repository(&self, path: &str) -> bool {
        self.repository_rules
            .iter()
            .any(|rule| rule.include.is_match(path) && !rule.exclude.is_match(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{Code, Manifest, Scope, ScopeExclude};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn package(id: &str, execution: &str, include: &str) -> RulePackage {
        let manifest = Manifest {
            documentation: crate::rule::Documentation {
                file: Some("rule.md".to_owned()),
                source: None,
            },
            schema_version: crate::CONTRACT_VERSION,
            id: id.to_owned(),
            title: id.to_owned(),
            mode: "advisory".to_owned(),
            severity: "warning".to_owned(),
            execution: execution.to_owned(),
            intent: None,
            scope: Scope {
                include: vec![include.to_owned()],
                exclude: Vec::<ScopeExclude>::new(),
                require_files: Vec::new(),
            },
            patterns: BTreeMap::new(),
            diagnostics: BTreeMap::new(),
            code: Code {
                language: "wt-rule-1".to_owned(),
                capabilities: Vec::new(),
                file: None,
                source: Some(String::new()),
            },
            tests_file: None,
            metadata: None,
        };
        RulePackage {
            qualified_id: format!("local/{id}"),
            scope_name: "local".to_owned(),
            directory: PathBuf::new(),
            manifest,
            manifest_value: serde_json::json!({}),
            source: String::new(),
            tests: None,
            fixture_contents: BTreeMap::new(),
            digest: String::new(),
        }
    }

    #[test]
    fn streamed_file_sources_are_not_retained() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("a.txt"), "abc").unwrap();
        let config = crate::config::merge(None, None, &serde_json::json!({})).unwrap();
        let file_rule = package("file", "file", "**/*.txt");
        let mut delivered = Vec::new();
        let selection = select_streamed(
            root.path(),
            &config,
            &serde_json::json!({"no_host_ignores": true}),
            &[&file_rule],
            |file| {
                delivered.push((file.path.clone(), file.text.clone(), file.digest.clone()));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].0, "a.txt");
        assert!(selection.files.is_empty());
        assert!(selection.repository_files.is_empty());
        assert_eq!(selection.source_reads, 1);
        assert_eq!(selection.bytes_hashed, 3);
        assert_eq!(selection.checked_files, 1);
    }

    #[test]
    fn streamed_repository_snapshots_are_scope_separate_from_file_inputs() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::create_dir(root.path().join("docs")).unwrap();
        fs::write(root.path().join("src/a.txt"), "a").unwrap();
        fs::write(root.path().join("docs/b.txt"), "b").unwrap();
        let config = crate::config::merge(None, None, &serde_json::json!({})).unwrap();
        let file_rule = package("file", "file", "src/**");
        let repository_rule = package("repo", "repository", "docs/**");
        let mut delivered = Vec::new();
        let selection = select_streamed(
            root.path(),
            &config,
            &serde_json::json!({"no_host_ignores": true}),
            &[&file_rule, &repository_rule],
            |file| {
                delivered.push(file.path.clone());
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(delivered, ["src/a.txt"]);
        assert_eq!(
            selection
                .repository_files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["docs/b.txt"]
        );
    }
}

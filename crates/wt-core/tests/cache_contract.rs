#[allow(dead_code)]
#[path = "../src/cache.rs"]
mod cache;
#[allow(dead_code)]
#[path = "../src/digest.rs"]
mod digest;
#[allow(dead_code)]
#[path = "../src/fixtures.rs"]
mod fixtures;
#[allow(dead_code)]
#[path = "../src/formatting.rs"]
mod formatting;
#[allow(dead_code)]
#[path = "../src/rule.rs"]
mod rule;

const CONTRACT_VERSION: u64 = 1;

use anyhow::Result;
use cache::{
    clear_at_directory, fixture_key_with_limits, raw_key_with_limits, repo_raw_key_with_limits,
    Store,
};
use wt_runtime::RuntimeLimits;

fn fixture_key(package: &rule::RulePackage) -> anyhow::Result<String> {
    fixture_key_with_limits(package, RuntimeLimits::default())
}

fn raw_key(package: &rule::RulePackage, path: &str, digest: &str) -> String {
    raw_key_with_limits(package, path, digest, RuntimeLimits::default())
}

fn repo_raw_key(package: &rule::RulePackage, files: &[(&str, &str)]) -> String {
    repo_raw_key_with_limits(package, files, RuntimeLimits::default())
}
use fixtures::{FixtureFile, TestCase, TestOutcome, TestSuite};
use rule::{Code, DiagnosticDefinition, Manifest, RulePackage, Scope, ScopeExclude};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::tempdir;
use wt_runtime::RawDiagnostic;

fn parse_json(input: &str) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::from_str(input)?)
}

fn package(directory: &Path, include: &str) -> RulePackage {
    let mut diagnostics = BTreeMap::new();
    diagnostics.insert(
        "hit".to_owned(),
        DiagnosticDefinition {
            kind: "violation".to_owned(),
            message: "hit".to_owned(),
            help: "fix".to_owned(),
        },
    );
    let manifest = Manifest {
        documentation: rule::Documentation {
            file: Some("rule.md".to_owned()),
            source: None,
        },
        schema_version: CONTRACT_VERSION,
        id: "cache-rule".to_owned(),
        title: "cache".to_owned(),
        mode: "advisory".to_owned(),
        severity: "warning".to_owned(),
        execution: "file".to_owned(),
        scope: Scope {
            include: vec![include.to_owned()],
            exclude: vec![ScopeExclude {
                glob: "generated/**".to_owned(),
                reason: "generated".to_owned(),
            }],
            require_files: Vec::new(),
        },
        patterns: BTreeMap::new(),
        diagnostics,
        code: Code {
            language: "wt-rule-1".to_owned(),
            capabilities: vec!["text.v1".to_owned()],
            file: Some("check.wt".to_owned()),
            source: None,
        },
        tests_file: Some("tests.json".to_owned()),
        metadata: None,
    };
    RulePackage {
        qualified_id: "local/cache-rule".to_owned(),
        scope_name: "local".to_owned(),
        directory: directory.to_path_buf(),
        manifest_value: serde_json::to_value(&manifest).unwrap(),
        source: "emit(file.span, \"hit\");".to_owned(),
        tests: Some(TestSuite {
            schema_version: CONTRACT_VERSION,
            cases: vec![TestCase {
                name: "fixture".to_owned(),
                files: vec![FixtureFile {
                    path: "input.txt".to_owned(),
                    content: None,
                    fixture: Some("fixtures/input.txt".to_owned()),
                }],
                expect: Vec::new(),
            }],
        }),
        fixture_contents: BTreeMap::from([("fixtures/input.txt".to_owned(), b"bad".to_vec())]),
        digest: "sha256:package".to_owned(),
        manifest,
    }
}

#[test]
fn raw_cache_filenames_are_portable_and_validated_against_source() {
    let directory = tempdir().unwrap();
    let mut store = Store::at_directory(directory.path());
    let key = "semantic-key";
    store.write_raw(
        key,
        &[RawDiagnostic {
            path: "src/input.txt".to_owned(),
            code: "hit".to_owned(),
            start_byte: 0,
            end_byte: 2,
        }],
    );
    let filename = fs::read_dir(directory.path().join("raw"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    assert!(!filename.to_string_lossy().contains(':'));
    assert!(store
        .read_raw_validated(key, "src/input.txt", "bad", [&"hit"])
        .is_some());

    store.write_raw(
        key,
        &[RawDiagnostic {
            path: "src/input.txt".to_owned(),
            code: "hit".to_owned(),
            start_byte: 1,
            end_byte: 2,
        }],
    );
    assert!(store
        .read_raw_validated(key, "src/input.txt", "é", [&"hit"])
        .is_none());
    assert!(store
        .notices
        .iter()
        .any(|notice| notice.contains("invalid raw cache entry")));
}

#[test]
fn cache_entry_reads_are_bounded_and_fixture_failures_are_not_persisted() {
    let directory = tempdir().unwrap();
    let mut store = Store::at_directory(directory.path());
    let key = "oversized";
    let digest = digest::digest_bytes(key.as_bytes());
    let filename = digest.strip_prefix("sha256:").unwrap();
    fs::write(
        directory
            .path()
            .join("raw")
            .join(format!("{filename}.json")),
        vec![b'x'; 16 * 1024 * 1024 + 1],
    )
    .unwrap();
    assert!(store.read_raw(key).is_none());
    assert!(store
        .notices
        .iter()
        .any(|notice| notice.contains("cache read failed")));

    let failed = TestOutcome {
        passed: false,
        cases: 1,
        failed_cases: vec!["case".to_owned()],
    };
    store.write_fixture("fixture", &failed);
    assert_eq!(store.stats.fixture_writes, 0);
    assert!(store.read_fixture("fixture").is_none());
}

#[test]
fn fixture_keys_use_loaded_bytes_and_package_identity() -> Result<()> {
    let first = tempdir()?;
    let second = tempdir()?;
    let first_package = package(first.path(), "**/*.txt");
    let second_package = package(second.path(), "**/*.txt");
    assert_ne!(fixture_key(&first_package)?, fixture_key(&second_package)?);

    let mut missing = first_package.clone();
    missing.fixture_contents.clear();
    assert!(fixture_key(&missing).is_err());

    let mut changed_scope = package(first.path(), "src/**/*.txt");
    assert_ne!(
        raw_key(&first_package, "src/input.txt", "sha256:file"),
        raw_key(&changed_scope, "src/input.txt", "sha256:file")
    );
    changed_scope.manifest.execution = "repository".to_owned();
    assert_ne!(
        raw_key(&first_package, "src/input.txt", "sha256:file"),
        raw_key(&changed_scope, "src/input.txt", "sha256:file")
    );

    let one_file = repo_raw_key(&first_package, &[("a.txt", "sha256:a")]);
    let two_files = repo_raw_key(
        &first_package,
        &[("a.txt", "sha256:a"), ("b.txt", "sha256:b")],
    );
    assert_ne!(one_file, two_files);
    let strict = RuntimeLimits {
        file_native_bytes: 1,
        ..RuntimeLimits::default()
    };
    assert_ne!(
        raw_key(&first_package, "src/input.txt", "sha256:file"),
        raw_key_with_limits(&first_package, "src/input.txt", "sha256:file", strict)
    );
    assert_ne!(
        fixture_key(&first_package)?,
        fixture_key_with_limits(&first_package, strict)?
    );
    assert_ne!(
        one_file,
        repo_raw_key_with_limits(&first_package, &[("a.txt", "sha256:a")], strict)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn clear_refuses_a_symlinked_cache_root() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(target.path().join("keep"), b"keep").unwrap();
    let link = directory.path().join("cache");
    symlink(target.path(), &link).unwrap();
    assert!(clear_at_directory(&link).is_err());
    assert!(target.path().join("keep").exists());
}

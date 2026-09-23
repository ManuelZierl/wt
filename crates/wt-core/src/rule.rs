use crate::digest::{
    digest_package_with_contents, normalize_reference, package_path, read_package_file,
    MAX_PACKAGE_FILE_BYTES,
};
use crate::fixtures::TestSuite;
use crate::parse_json;
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_SOURCE_BYTES: usize = 128 * 1024;
const MAX_TEST_BYTES: usize = 4 * 1024 * 1024;
static RULE_ID: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<ScopeExclude>,
    #[serde(default)]
    pub require_files: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeExclude {
    pub glob: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternDefinition {
    pub regex: String,
    #[serde(default)]
    pub flags: PatternFlags,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternFlags {
    #[serde(default)]
    pub case_insensitive: bool,
    #[serde(default)]
    pub multi_line: bool,
    #[serde(default)]
    pub dot_matches_new_line: bool,
    #[serde(default)]
    pub ignore_whitespace: bool,
    #[serde(default)]
    pub crlf: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Pattern {
    Shorthand(String),
    Explicit(PatternDefinition),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticDefinition {
    pub kind: String,
    pub message: String,
    pub help: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Code {
    pub language: String,
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Long-form rule prose has exactly one authoritative representation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Documentation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u64,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Documentation>,
    #[serde(default = "default_mode")]
    pub mode: String,
    pub severity: String,
    #[serde(default = "default_execution")]
    pub execution: String,
    pub scope: Scope,
    #[serde(default)]
    pub patterns: BTreeMap<String, Pattern>,
    pub diagnostics: BTreeMap<String, DiagnosticDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    pub code: Code,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub schema_version: u64,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Documentation>,
    #[serde(default = "default_mode")]
    pub mode: String,
    pub severity: String,
    #[serde(default = "default_execution")]
    pub execution: String,
    pub scope: Scope,
    #[serde(default)]
    pub patterns: BTreeMap<String, Pattern>,
    pub diagnostics: BTreeMap<String, DiagnosticDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    pub code: Code,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests: Option<TestSuite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Clone)]
pub struct RulePackage {
    pub qualified_id: String,
    pub scope_name: String,
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub manifest_value: Value,
    pub source: String,
    pub tests: Option<TestSuite>,
    pub(crate) fixture_contents: BTreeMap<String, Vec<u8>>,
    pub digest: String,
}

pub fn default_mode() -> String {
    "advisory".to_owned()
}

pub fn default_execution() -> String {
    "file".to_owned()
}

pub fn valid_rule_id(id: &str) -> bool {
    RULE_ID
        .get_or_init(|| Regex::new(r"^[a-z][a-z0-9]*(?:[-.][a-z0-9]+)*$").unwrap())
        .is_match(id)
}

pub fn safe_relative(path: &str) -> bool {
    if path.is_empty() || path.contains('\\') || Path::new(path).is_absolute() {
        return false;
    }
    let mut saw_component = false;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) => saw_component = true,
            Component::CurDir => {}
            _ => return false,
        }
    }
    saw_component
}

pub fn validate_submission(submission: &Submission) -> Result<()> {
    validate_documentation(
        submission.schema_version,
        &submission.title,
        &submission.description,
        &submission.rationale,
        &submission.limitations,
        submission.documentation.as_ref(),
        true,
    )?;
    validate_common(
        ValidationFields {
            id: &submission.id,
            mode: &submission.mode,
            severity: &submission.severity,
            execution: &submission.execution,
            scope: &submission.scope,
            patterns: &submission.patterns,
            diagnostics: &submission.diagnostics,
            code: &submission.code,
        },
        true,
    )?;
    if submission.code.source.is_none() || submission.code.file.is_some() {
        bail!("submission code must contain source and must not contain file")
    }
    if let Some(tests) = &submission.tests {
        tests.validate()?;
        for case in &tests.cases {
            if case.files.iter().any(|file| file.fixture.is_some()) {
                bail!("submissions may contain inline fixture content only")
            }
            for expected in &case.expect {
                let diagnostic = submission.diagnostics.get(&expected.code).ok_or_else(|| {
                    anyhow!("fixture references unknown diagnostic {}", expected.code)
                })?;
                if diagnostic.kind != expected.kind {
                    bail!("fixture diagnostic kind does not match manifest")
                }
            }
        }
    }
    Ok(())
}

pub fn validate_manifest(manifest: &Manifest) -> Result<()> {
    validate_documentation(
        manifest.schema_version,
        &manifest.title,
        &manifest.description,
        &manifest.rationale,
        &manifest.limitations,
        manifest.documentation.as_ref(),
        false,
    )?;
    validate_common(
        ValidationFields {
            id: &manifest.id,
            mode: &manifest.mode,
            severity: &manifest.severity,
            execution: &manifest.execution,
            scope: &manifest.scope,
            patterns: &manifest.patterns,
            diagnostics: &manifest.diagnostics,
            code: &manifest.code,
        },
        false,
    )?;
    if manifest.code.file.as_deref() != Some("check.wt") || manifest.code.source.is_some() {
        bail!("disk manifest code must reference check.wt and contain no inline source")
    }
    if let Some(path) = &manifest.tests_file {
        if !safe_relative(path) {
            bail!("tests_file is not a safe package-relative path")
        }
    }
    Ok(())
}

struct ValidationFields<'a> {
    id: &'a str,
    mode: &'a str,
    severity: &'a str,
    execution: &'a str,
    scope: &'a Scope,
    patterns: &'a BTreeMap<String, Pattern>,
    diagnostics: &'a BTreeMap<String, DiagnosticDefinition>,
    code: &'a Code,
}

fn validate_common(fields: ValidationFields<'_>, submission: bool) -> Result<()> {
    let ValidationFields {
        id,
        mode,
        severity,
        execution,
        scope,
        patterns,
        diagnostics,
        code,
    } = fields;
    if !valid_rule_id(id) {
        bail!("invalid rule id {id:?}")
    }
    if !matches!(mode, "advisory" | "enforced" | "disabled") {
        bail!("invalid rule mode {mode:?}")
    }
    if !matches!(severity, "error" | "warning" | "info") {
        bail!("invalid rule severity {severity:?}")
    }
    if !matches!(execution, "file" | "repository") {
        bail!("invalid execution mode {execution:?}")
    }
    if scope.include.is_empty() || scope.include.iter().any(|glob| !valid_glob(glob)) {
        bail!("scope.include must contain safe, valid globs")
    }
    for entry in &scope.exclude {
        if !valid_glob(&entry.glob) || entry.reason.trim().is_empty() {
            bail!("scope exclusions require safe globs and nonempty reasons")
        }
    }
    for path in &scope.require_files {
        if !safe_relative(path) {
            bail!("scope.require_files contains an unsafe path")
        }
    }
    if patterns.len() > 128 {
        bail!("a rule may declare at most 128 patterns")
    }
    for (name, pattern) in patterns {
        if !valid_identifier(name) {
            bail!("invalid pattern id {name:?}")
        }
        let regex = match pattern {
            Pattern::Shorthand(value) => value,
            Pattern::Explicit(value) => &value.regex,
        };
        if regex.len() > 8 * 1024 {
            bail!("pattern {name:?} exceeds 8 KiB")
        }
        let definition = match pattern {
            Pattern::Shorthand(_) => PatternDefinition {
                regex: regex.clone(),
                flags: PatternFlags::default(),
            },
            Pattern::Explicit(value) => value.clone(),
        };
        let mut builder = regex::RegexBuilder::new(&definition.regex);
        builder
            .case_insensitive(definition.flags.case_insensitive)
            .multi_line(definition.flags.multi_line)
            .dot_matches_new_line(definition.flags.dot_matches_new_line)
            .ignore_whitespace(definition.flags.ignore_whitespace)
            .crlf(definition.flags.crlf);
        builder
            .build()
            .map_err(|error| anyhow!("invalid regex {name:?}: {error}"))?;
    }
    if diagnostics.is_empty() {
        bail!("diagnostics must not be empty")
    }
    for (id, diagnostic) in diagnostics {
        if !valid_diagnostic_id(id) {
            bail!("invalid diagnostic id {id:?}")
        }
        if !matches!(diagnostic.kind.as_str(), "violation" | "review")
            || diagnostic.message.trim().is_empty()
            || diagnostic.help.trim().is_empty()
        {
            bail!("diagnostic {id:?} has invalid kind or empty message/help")
        }
    }
    if code.language != "wt-rule-1" {
        bail!("code.language must be wt-rule-1")
    }
    if code.capabilities.iter().any(|cap| {
        !matches!(
            cap.as_str(),
            "text.v1" | "regex.v1" | "path.v1" | "jsx.v1" | "repo.v1"
        )
    }) {
        bail!("code.capabilities contains an unknown capability")
    }
    if !submission && code.source.is_some() {
        bail!("disk manifests cannot contain code.source")
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn valid_diagnostic_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn valid_glob(value: &str) -> bool {
    if value.is_empty() || value.contains('\\') || value.starts_with('/') {
        return false;
    }
    if value.split('/').any(|component| component == "..") {
        return false;
    }
    globset::GlobBuilder::new(value)
        .literal_separator(true)
        .build()
        .is_ok()
}

pub fn submission_from_value(value: Value) -> Result<Submission> {
    validate_prose_fields(&value)?;
    let submission: Submission =
        serde_json::from_value(value).context("invalid rule submission")?;
    validate_submission(&submission)?;
    Ok(submission)
}

pub fn manifest_from_value(value: Value) -> Result<Manifest> {
    validate_prose_fields(&value)?;
    let manifest: Manifest = serde_json::from_value(value).context("invalid rule manifest")?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn load_package(directory: &Path, scope_name: &str) -> Result<RulePackage> {
    let manifest_path = directory.join("rule.json");
    if fs::symlink_metadata(&manifest_path)?
        .file_type()
        .is_symlink()
    {
        bail!("rule.json must not be a symlink")
    }
    let manifest_bytes = read_package_file(directory, "rule.json", MAX_PACKAGE_FILE_BYTES)
        .with_context(|| format!("unable to read {}", manifest_path.display()))?;
    let manifest_value = parse_json(std::str::from_utf8(&manifest_bytes)?)?;
    let manifest = manifest_from_value(manifest_value.clone())?;
    let directory_name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if directory_name != manifest.id && !directory_name.starts_with(".wt-rule-") {
        bail!("rule directory name does not match manifest id")
    }
    let code_path = directory.join("check.wt");
    ensure_reference_inside(directory, "check.wt")?;
    if fs::symlink_metadata(&code_path)?.file_type().is_symlink() {
        bail!("check.wt must not be a symlink")
    }
    let source_bytes = read_package_file(directory, "check.wt", MAX_SOURCE_BYTES)
        .context("unable to read check.wt")?;
    if source_bytes.len() > MAX_SOURCE_BYTES {
        bail!("check.wt exceeds 128 KiB")
    }
    let source = String::from_utf8(source_bytes).context("check.wt is not UTF-8")?;
    let mut package_contents = BTreeMap::new();
    package_contents.insert("check.wt".to_owned(), source.as_bytes().to_owned());
    let tests = if let Some(tests_path) = manifest.tests_file.as_deref() {
        ensure_reference_inside(directory, tests_path)?;
        let tests_path_on_disk = directory.join(tests_path);
        if !tests_path_on_disk.is_file() {
            bail!("declared tests_file does not exist")
        }
        if fs::symlink_metadata(&tests_path_on_disk)?
            .file_type()
            .is_symlink()
        {
            bail!("test suite must not be a symlink")
        }
        let bytes = read_package_file(directory, tests_path, MAX_TEST_BYTES)?;
        if bytes.len() > MAX_TEST_BYTES {
            bail!("test suite exceeds 4 MiB")
        }
        let value = parse_json(std::str::from_utf8(&bytes)?)?;
        let suite = TestSuite::from_value(value)?;
        package_contents.insert(normalize_reference(tests_path)?, bytes);
        for case in &suite.cases {
            for expected in &case.expect {
                let diagnostic = manifest.diagnostics.get(&expected.code).ok_or_else(|| {
                    anyhow!("fixture references unknown diagnostic {}", expected.code)
                })?;
                if diagnostic.kind != expected.kind {
                    bail!("fixture diagnostic kind does not match manifest")
                }
            }
            for fixture in &case.files {
                if let Some(path) = &fixture.fixture {
                    ensure_reference_inside(directory, path)?;
                    let bytes = read_package_file(directory, path, MAX_PACKAGE_FILE_BYTES)?;
                    package_contents.insert(normalize_reference(path)?, bytes);
                }
            }
        }
        Some(suite)
    } else {
        None
    };
    if manifest.mode == "enforced" {
        let suite = tests
            .as_ref()
            .ok_or_else(|| anyhow!("enforced rule requires a test suite"))?;
        if !suite.has_applicable_positive_and_negative(&manifest, &package_contents) {
            bail!("enforced rule requires applicable positive and nonempty negative fixtures")
        }
    }
    let mut references = vec!["check.wt".to_owned()];
    if manifest.documentation.is_some() {
        ensure_reference_inside(directory, "rule.md")?;
        if fs::symlink_metadata(directory.join("rule.md"))?
            .file_type()
            .is_symlink()
        {
            bail!("rule.md must not be a symlink");
        }
        let bytes = read_package_file(directory, "rule.md", MAX_SOURCE_BYTES)?;
        if std::str::from_utf8(&bytes)?.trim().is_empty() {
            bail!("rule.md must contain nonempty UTF-8 documentation");
        }
        package_contents.insert("rule.md".to_owned(), bytes);
        references.push("rule.md".to_owned());
    }
    if let Some(path) = &manifest.tests_file {
        references.push(path.clone());
        if let Some(suite) = &tests {
            references.extend(
                suite
                    .cases
                    .iter()
                    .flat_map(|case| case.files.iter().filter_map(|file| file.fixture.clone())),
            );
        }
    }
    let reference_values = references.iter().map(String::as_str).collect::<Vec<_>>();
    let digest = digest_package_with_contents(
        directory,
        &reference_values,
        &manifest_bytes,
        &package_contents,
    )?;
    Ok(RulePackage {
        qualified_id: format!("{scope_name}/{}", manifest.id),
        scope_name: scope_name.to_owned(),
        directory: directory.to_path_buf(),
        manifest,
        manifest_value,
        source,
        tests,
        fixture_contents: package_contents,
        digest,
    })
}

pub fn ensure_reference_inside(package: &Path, reference: &str) -> Result<()> {
    if !safe_relative(reference) {
        bail!("unsafe package reference {reference:?}")
    }
    let _ = package_path(package, reference)?;
    Ok(())
}

pub fn submission_manifest(submission: &Submission) -> (Manifest, TestSuite) {
    let manifest = Manifest {
        schema_version: submission.schema_version,
        id: submission.id.clone(),
        title: submission.title.clone(),
        description: submission.description.clone(),
        rationale: submission.rationale.clone(),
        documentation: submission.documentation.as_ref().map(|_| Documentation {
            file: Some("rule.md".to_owned()),
            source: None,
        }),
        mode: submission.mode.clone(),
        severity: submission.severity.clone(),
        execution: submission.execution.clone(),
        scope: submission.scope.clone(),
        patterns: submission.patterns.clone(),
        diagnostics: submission.diagnostics.clone(),
        limitations: submission.limitations.clone(),
        code: Code {
            language: submission.code.language.clone(),
            capabilities: submission.code.capabilities.clone(),
            file: Some("check.wt".to_owned()),
            source: None,
        },
        tests_file: submission.tests.as_ref().map(|_| "tests.json".to_owned()),
        metadata: submission.metadata.clone(),
    };
    (manifest, submission.tests.clone().unwrap_or_default())
}

pub fn disk_manifest_value(manifest: &Manifest) -> Value {
    serde_json::to_value(manifest).expect("manifest serialization")
}

pub fn export_value(package: &RulePackage) -> Result<Value> {
    let mut object = match package.manifest_value.clone() {
        Value::Object(value) => value,
        _ => bail!("manifest is not an object"),
    };
    let mut code = object
        .remove("code")
        .ok_or_else(|| anyhow!("manifest code is missing"))?;
    code["source"] = Value::String(package.source.clone());
    if let Value::Object(ref mut code_object) = code {
        code_object.remove("file");
    }
    object.insert("code".to_owned(), code);
    if package.manifest.documentation.is_some() {
        let bytes = package
            .fixture_contents
            .get("rule.md")
            .ok_or_else(|| anyhow!("rule.md was not loaded"))?;
        object.insert(
            "documentation".to_owned(),
            serde_json::json!({
                "source": std::str::from_utf8(bytes)?
            }),
        );
    }
    if let Some(tests) = &package.tests {
        let mut tests_value = serde_json::to_value(tests)?;
        let cases = tests_value
            .get_mut("cases")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow!("serialized test suite is not an object"))?;
        for case in cases {
            let files = case
                .get_mut("files")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| anyhow!("serialized fixture case is not an object"))?;
            for file in files {
                let Some(path) = file
                    .get("fixture")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                else {
                    continue;
                };
                let normalized = normalize_reference(&path)?;
                let bytes = package
                    .fixture_contents
                    .get(&normalized)
                    .ok_or_else(|| anyhow!("fixture reference {path:?} was not loaded"))?;
                let content = String::from_utf8(bytes.clone()).context("fixture is not UTF-8")?;
                let file_object = file
                    .as_object_mut()
                    .ok_or_else(|| anyhow!("serialized fixture file is not an object"))?;
                file_object.remove("fixture");
                file_object.insert("content".to_owned(), Value::String(content));
            }
        }
        object.insert("tests".to_owned(), tests_value);
    }
    object.remove("tests_file");
    Ok(Value::Object(object))
}

pub fn map_with_code_source(submission: &Submission) -> Value {
    serde_json::to_value(submission).expect("submission serialization")
}

fn validate_prose_fields(value: &Value) -> Result<()> {
    if value["schema_version"] == 3
        && ["description", "rationale", "limitations"]
            .iter()
            .any(|key| value.get(key).is_some())
    {
        bail!("schema 3 stores prose only in documentation; remove legacy description/rationale/limitations");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_documentation(
    version: u64,
    title: &str,
    description: &str,
    rationale: &str,
    limitations: &[String],
    documentation: Option<&Documentation>,
    submission: bool,
) -> Result<()> {
    if title.trim().is_empty() {
        bail!("title must not be empty");
    }
    match (version, documentation) {
        (2, None) if !description.trim().is_empty() && !rationale.trim().is_empty() => Ok(()),
        (3, Some(doc))
            if description.is_empty() && rationale.is_empty() && limitations.is_empty() =>
        {
            if submission {
                if doc.file.is_some()
                    || !doc
                        .source
                        .as_ref()
                        .is_some_and(|s| !s.trim().is_empty() && s.len() <= MAX_SOURCE_BYTES)
                {
                    bail!("documentation requires nonempty source (at most 128 KiB), not file");
                }
            } else if doc.file.as_deref() != Some("rule.md") || doc.source.is_some() {
                bail!("disk documentation must reference rule.md, without source");
            }
            Ok(())
        }
        _ => bail!("use legacy schema 2 prose, or schema 3 documentation without duplicate prose"),
    }
}

/// Canonical storage is schema 3; legacy input remains importable without
/// discarding its prose. Detector fixtures are retained verbatim.
pub fn prepare_submission(mut submission: Submission) -> Result<Submission> {
    validate_submission(&submission)?;
    let manifest = map_with_code_source(&submission);
    let source = submission.code.source.as_deref().unwrap_or_default();
    wt_runtime::compile(&manifest, source)?;
    let formatted = crate::formatting::format_source(source)?;
    wt_runtime::compile(&manifest, &formatted)?;
    submission.code.source = Some(formatted);
    if submission.schema_version == 2 {
        let mut markdown = format!(
            "# {}\n\n## Description\n\n{}\n\n## Rationale\n\n{}\n\n## Limitations\n\n",
            submission.title, submission.description, submission.rationale
        );
        if submission.limitations.is_empty() {
            markdown.push_str("No limitations were recorded in the legacy package.\n");
        } else {
            for limitation in &submission.limitations {
                markdown.push_str(&format!("- {limitation}\n"));
            }
        }
        submission.documentation = Some(Documentation {
            source: Some(markdown),
            file: None,
        });
        submission.schema_version = 3;
        submission.description.clear();
        submission.rationale.clear();
        submission.limitations.clear();
    }
    validate_submission(&submission)?;
    Ok(submission)
}

use crate::digest::normalize_reference;
use crate::rule::{safe_relative, Manifest, RulePackage};
use anyhow::{anyhow, bail, Result};
use globset::{GlobBuilder, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wt_runtime::{RawDiagnostic, SourceFile};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestSuite {
    pub schema_version: u64,
    pub cases: Vec<TestCase>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestCase {
    pub name: String,
    pub files: Vec<FixtureFile>,
    #[serde(default)]
    pub expect: Vec<ExpectedDiagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExpectedDiagnostic {
    pub path: String,
    pub code: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_byte: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_byte: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestOutcome {
    pub passed: bool,
    pub cases: usize,
    pub failed_cases: Vec<String>,
}

impl TestSuite {
    pub fn from_value(value: Value) -> Result<Self> {
        let suite: Self = serde_json::from_value(value).map_err(|error| anyhow!(error))?;
        suite.validate()?;
        Ok(suite)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 2 {
            bail!("test suite schema_version must be 2")
        }
        let mut names = std::collections::HashSet::new();
        for case in &self.cases {
            if case.name.trim().is_empty() || !names.insert(case.name.clone()) {
                bail!("fixture case names must be unique and nonempty")
            }
            if case.files.is_empty() {
                bail!("fixture case {:?} has no files", case.name)
            }
            let mut paths = std::collections::HashSet::new();
            for file in &case.files {
                if !safe_relative(&file.path) || !paths.insert(file.path.clone()) {
                    bail!("fixture paths must be safe and unique")
                }
                if file.content.is_some() == file.fixture.is_some() {
                    bail!(
                        "fixture file {:?} requires exactly one of content or fixture",
                        file.path
                    )
                }
                if let Some(path) = &file.fixture {
                    if !safe_relative(path) {
                        bail!("fixture reference {:?} is unsafe", path)
                    }
                }
            }
            for expected in &case.expect {
                if !safe_relative(&expected.path)
                    || !matches!(expected.kind.as_str(), "violation" | "review")
                    || expected.code.trim().is_empty()
                {
                    bail!("invalid expected diagnostic in case {:?}", case.name)
                }
                if expected.start_byte.is_some() != expected.end_byte.is_some() {
                    bail!("expected diagnostic spans must specify both byte offsets")
                }
                if !case.files.iter().any(|file| file.path == expected.path) {
                    bail!("expected diagnostic path is not a fixture file")
                }
            }
        }
        Ok(())
    }

    pub fn has_applicable_positive_and_negative(
        &self,
        manifest: &Manifest,
        package_contents: &std::collections::BTreeMap<String, Vec<u8>>,
    ) -> bool {
        let Ok(globs) = compile_globs(manifest) else {
            return false;
        };
        let positive = self.cases.iter().any(|case| {
            if case.expect.is_empty() {
                return false;
            }
            let applicable = applicable_files(case, manifest, &globs);
            case.expect
                .iter()
                .any(|expected| applicable.iter().any(|file| file.path == expected.path))
        });
        let negative = self.cases.iter().any(|case| {
            if !case.expect.is_empty() {
                return false;
            }
            applicable_files(case, manifest, &globs).iter().any(|file| {
                fixture_bytes(file, package_contents).is_some_and(|bytes| !bytes.is_empty())
            })
        });
        positive && negative
    }
}

pub fn run_suite_with<F>(package: &RulePackage, mut execute: F) -> Result<TestOutcome>
where
    F: FnMut(&[SourceFile]) -> Result<Vec<RawDiagnostic>>,
{
    let suite = package
        .tests
        .as_ref()
        .ok_or_else(|| anyhow!("missing_test_suite"))?;
    let mut failed_cases = Vec::new();
    let include = compile_globs(&package.manifest)?;
    for case in &suite.cases {
        let mut files = Vec::new();
        for fixture in &case.files {
            let text = match (&fixture.content, &fixture.fixture) {
                (Some(content), None) => content.clone(),
                (None, Some(path)) => {
                    let normalized = normalize_reference(path)?;
                    let bytes = package
                        .fixture_contents
                        .get(&normalized)
                        .ok_or_else(|| anyhow!("fixture reference {path:?} was not loaded"))?;
                    String::from_utf8(bytes.clone()).map_err(|_| anyhow!("fixture is not UTF-8"))?
                }
                _ => bail!("invalid fixture file"),
            };
            files.push(SourceFile {
                path: fixture.path.clone(),
                text: text.into(),
            });
        }
        let has_requirements = package
            .manifest
            .scope
            .require_files
            .iter()
            .all(|required| files.iter().any(|file| file.path == *required));
        let applicable = files
            .iter()
            .filter(|file| has_requirements && scope_matches(&include, &file.path))
            .cloned()
            .collect::<Vec<_>>();
        let input = if package.manifest.execution == "repository" {
            applicable.clone()
        } else {
            Vec::new()
        };
        let actual = if package.manifest.execution == "repository" {
            if applicable.is_empty() {
                Vec::new()
            } else {
                execute(&input)?
            }
        } else {
            let mut output = Vec::new();
            for file in applicable {
                output.extend(execute(&[file])?);
            }
            output
        };
        if !matches_expected(&actual, &case.expect, &package.manifest) {
            failed_cases.push(case.name.clone());
        }
    }
    Ok(TestOutcome {
        passed: failed_cases.is_empty(),
        cases: suite.cases.len(),
        failed_cases,
    })
}

fn compile_globs(manifest: &Manifest) -> Result<(globset::GlobSet, globset::GlobSet)> {
    let mut includes = GlobSetBuilder::new();
    for value in &manifest.scope.include {
        includes.add(GlobBuilder::new(value).literal_separator(true).build()?);
    }
    let mut excludes = GlobSetBuilder::new();
    for value in &manifest.scope.exclude {
        excludes.add(
            GlobBuilder::new(&value.glob)
                .literal_separator(true)
                .build()?,
        );
    }
    Ok((includes.build()?, excludes.build()?))
}

fn scope_matches(globs: &(globset::GlobSet, globset::GlobSet), path: &str) -> bool {
    globs.0.is_match(path) && !globs.1.is_match(path)
}

fn applicable_files<'a>(
    case: &'a TestCase,
    manifest: &Manifest,
    globs: &(globset::GlobSet, globset::GlobSet),
) -> Vec<&'a FixtureFile> {
    if !manifest
        .scope
        .require_files
        .iter()
        .all(|required| case.files.iter().any(|file| file.path == *required))
    {
        return Vec::new();
    }
    case.files
        .iter()
        .filter(|file| scope_matches(globs, &file.path))
        .collect()
}

fn fixture_bytes<'a>(
    file: &'a FixtureFile,
    package_contents: &'a std::collections::BTreeMap<String, Vec<u8>>,
) -> Option<&'a [u8]> {
    match (&file.content, &file.fixture) {
        (Some(content), None) => Some(content.as_bytes()),
        (None, Some(path)) => normalize_reference(path)
            .ok()
            .and_then(|path| package_contents.get(&path).map(Vec::as_slice)),
        _ => None,
    }
}

fn matches_expected(
    actual: &[RawDiagnostic],
    expected: &[ExpectedDiagnostic],
    manifest: &Manifest,
) -> bool {
    let mut remaining = actual
        .iter()
        .map(|diagnostic| ExpectedDiagnostic {
            path: diagnostic.path.clone(),
            code: diagnostic.code.clone(),
            kind: manifest.diagnostics.get(&diagnostic.code).map_or_else(
                || "violation".to_owned(),
                |definition| definition.kind.clone(),
            ),
            start_byte: Some(diagnostic.start_byte),
            end_byte: Some(diagnostic.end_byte),
        })
        .collect::<Vec<_>>();
    let mut wanted_diagnostics = expected.to_vec();
    // Pair constrained spans first so a wildcard cannot consume the only
    // actual diagnostic that satisfies a later exact-span expectation.
    wanted_diagnostics.sort_by_key(|wanted| wanted.start_byte.is_none());
    for wanted in &wanted_diagnostics {
        let Some(index) = remaining.iter().position(|found| {
            found.path == wanted.path
                && found.code == wanted.code
                && found.kind == wanted.kind
                && wanted
                    .start_byte
                    .zip(wanted.end_byte)
                    .is_none_or(|(start, end)| {
                        found.start_byte == Some(start) && found.end_byte == Some(end)
                    })
        }) else {
            return false;
        };
        remaining.remove(index);
    }
    remaining.is_empty() && expected.len() == actual.len()
}

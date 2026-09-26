use clap::{error::ErrorKind, CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

mod text;

const MAX_JSON_INPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Detail {
    Summary,
    Full,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Optimizer {
    Auto,
    Off,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Mode {
    Advisory,
    Enforced,
    Disabled,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum SchemaName {
    Rule,
    Submission,
    Tests,
    Config,
    Result,
    Plan,
    Review,
    Capabilities,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum ReviewDecision {
    NeedsReview,
    Acceptable,
    ConfirmedIssue,
    AcceptedRisk,
}

#[derive(Clone, Debug, Parser)]
#[command(name = "wt", version, about = "Watchtower repository checks")]
pub struct Cli {
    #[command(flatten)]
    pub common: Common,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Debug, clap::Args)]
pub struct Common {
    #[arg(long, global = true, value_name = "PATH")]
    pub root: Option<PathBuf>,
    #[arg(long = "global-dir", global = true, value_name = "PATH")]
    pub global_dir: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
    #[arg(long, global = true, value_enum, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Report installed build, supported schemas and offline guide digests.
    Capabilities,
    /// Show release-matched offline guidance.
    Guide {
        #[arg(value_name = "TOPIC")]
        topic: Option<String>,
    },
    /// Format local rule programs without changing detector behavior.
    Fmt {
        #[arg(value_name = "ID")]
        id: Option<String>,
        /// Inspect without writing; exit 1 when formatting differs.
        #[arg(long)]
        check: bool,
        /// Format user-global packages instead of local packages.
        #[arg(long)]
        global: bool,
    },
    /// Record one explicit, evidence-bound occurrence decision.
    Review {
        #[arg(value_name = "FINDING_ID")]
        finding_id: String,
        #[arg(long, value_enum)]
        decision: ReviewDecision,
        /// Evidence digest printed by the check that was actually reviewed.
        #[arg(long, required = true, value_name = "HASH")]
        expect_evidence: String,
        /// Required when appending to existing review history.
        #[arg(long, value_name = "HASH")]
        expect_hash: Option<String>,
        /// Markdown explanation of this decision and its assumptions.
        #[arg(long, required = true, value_name = "PATH")]
        reason_file: PathBuf,
        /// Additional reviewed evidence, including its expected content hash.
        #[arg(long, value_name = "PATH=sha256:HASH")]
        watch: Vec<String>,
        #[arg(long = "rule", value_name = "ID")]
        rules: Vec<String>,
        #[arg(long)]
        no_global: bool,
        #[arg(long)]
        no_host_ignores: bool,
    },
    /// Inspect stored review decisions; does not claim they are current.
    Reviews {
        #[arg(value_name = "FINDING_ID")]
        finding_id: Option<String>,
    },
    /// Evaluate one finding against current source and retained rationale.
    Inspect {
        #[arg(value_name = "FINDING_ID")]
        finding_id: String,
        #[arg(long)]
        no_global: bool,
        #[arg(long)]
        no_host_ignores: bool,
    },

    /// Initialize local or global Watchtower configuration.
    Init {
        #[arg(long)]
        global: bool,
    },
    /// Validate, format and create a readable rule package from JSON.
    New {
        #[arg(value_name = "JSON")]
        json: Option<String>,
        #[arg(long, conflicts_with_all = ["json", "file"])]
        stdin: bool,
        #[arg(long, value_name = "PATH", conflicts_with_all = ["json", "stdin"])]
        file: Option<PathBuf>,
        #[arg(long)]
        global: bool,
    },
    /// Run selected rules and apply current occurrence decisions.
    Check {
        #[arg(long, value_enum, default_value_t = Detail::Summary)]
        detail: Detail,
        #[arg(long, value_name = "PATH", conflicts_with = "rules")]
        submission: Option<PathBuf>,
        #[arg(value_name = "PATH")]
        paths: Vec<String>,
        #[arg(long = "include-ignored")]
        include_ignored: bool,
        #[arg(long = "no-host-ignores")]
        no_host_ignores: bool,
        #[arg(long = "no-global")]
        no_global: bool,
        #[arg(long = "rule", value_name = "ID")]
        rules: Vec<String>,
        #[arg(long)]
        strict: bool,
        #[arg(long = "no-cache")]
        no_cache: bool,
        #[arg(long, value_enum)]
        optimizer: Option<Optimizer>,
        #[arg(long)]
        changed: bool,
        #[arg(long)]
        base: Option<String>,
        #[arg(long, value_name = "N")]
        jobs: Option<u64>,
        #[arg(long = "max-file-bytes", value_name = "N")]
        max_file_bytes: Option<u64>,
        #[arg(long = "show-reviewed")]
        show_reviewed: bool,
        #[arg(long = "allow-empty")]
        allow_empty: bool,
        #[arg(long)]
        stats: bool,
    },
    /// Report per-rule findings and review outcomes to spot noisy and dead rules.
    Stats {
        #[arg(value_name = "PATH")]
        paths: Vec<String>,
        #[arg(long = "include-ignored")]
        include_ignored: bool,
        #[arg(long = "no-host-ignores")]
        no_host_ignores: bool,
        #[arg(long = "no-global")]
        no_global: bool,
        #[arg(long = "rule", value_name = "ID")]
        rules: Vec<String>,
        #[arg(long = "no-cache")]
        no_cache: bool,
        #[arg(long, value_enum)]
        optimizer: Option<Optimizer>,
        #[arg(long)]
        changed: bool,
        #[arg(long)]
        base: Option<String>,
        #[arg(long, value_name = "N")]
        jobs: Option<u64>,
        #[arg(long = "max-file-bytes", value_name = "N")]
        max_file_bytes: Option<u64>,
    },
    /// Inspect shared query planning without checking source files.
    Plan {
        #[arg(long = "rule", value_name = "ID")]
        rules: Vec<String>,
        #[arg(long = "no-global")]
        no_global: bool,
        #[arg(long, value_enum)]
        optimizer: Option<Optimizer>,
    },
    /// List rules, effective modes and package digests.
    List {
        #[arg(long = "no-global")]
        no_global: bool,
    },
    /// Export a self-contained rule and its current package digest.
    Show {
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Validate source and metadata; use test for fixture execution.
    Validate {
        #[arg(value_name = "ID")]
        id: Option<String>,
        #[arg(long, value_name = "PATH", conflicts_with = "id")]
        file: Option<PathBuf>,
        #[arg(long = "no-global")]
        no_global: bool,
    },
    /// Run retained detector fixtures independently of review state.
    Test {
        #[arg(value_name = "ID")]
        id: Option<String>,
        #[arg(long = "no-global")]
        no_global: bool,
    },
    /// Atomically update a rule, preserving regression evidence.
    Update {
        #[arg(value_name = "ID")]
        id: String,
        #[arg(long, required = true)]
        stdin: bool,
        #[arg(long = "expect-hash", value_name = "HASH", required = true)]
        expect_hash: String,
        #[arg(long = "allow-test-removal")]
        allow_test_removal: bool,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        preview: bool,
    },
    /// Explicitly change whether findings are advisory or blocking.
    SetMode {
        #[arg(value_name = "ID")]
        id: String,
        #[arg(value_enum, value_name = "MODE")]
        mode: Mode,
        #[arg(long, required = true)]
        reason: String,
        #[arg(long = "no-global")]
        no_global: bool,
    },
    /// Explain why a source path is selected or excluded.
    Explain {
        #[arg(value_name = "PATH")]
        path: String,
        #[arg(long = "rule", value_name = "ID")]
        rules: Vec<String>,
        #[arg(long = "no-global")]
        no_global: bool,
        #[arg(long = "include-ignored")]
        include_ignored: bool,
        #[arg(long = "no-host-ignores")]
        no_host_ignores: bool,
    },
    /// Display configuration values and their origins.
    Config {
        #[arg(long = "no-global")]
        no_global: bool,
        #[arg(long = "max-file-bytes", value_name = "N")]
        max_file_bytes: Option<u64>,
    },
    /// Print a bundled machine-readable JSON schema.
    Schema {
        #[arg(value_enum, value_name = "NAME")]
        name: SchemaName,
    },
    /// Manage derived caches; review records are never cache entries.
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
}

#[derive(Clone, Debug, Subcommand)]
pub enum CacheCommand {
    Clear,
}

pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    match Cli::try_parse_from(&args) {
        Ok(cli) => execute(cli),
        Err(error) => {
            if json_hint(&args)
                && !matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                )
            {
                let command = command_name(&args);
                print_json(&command_error(&command, error.to_string()));
                2
            } else {
                let code = if matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                ) {
                    0
                } else {
                    2
                };
                eprint!("{error}");
                code
            }
        }
    }
}

fn execute(cli: Cli) -> i32 {
    let command = command_string(&cli.command);
    let display = match &cli.command {
        Command::Check { show_reviewed, .. } => *show_reviewed,
        _ => false,
    };
    if let Command::Schema { name } = &cli.command {
        return print_bundled_schema(name);
    }
    let options = match options(&cli) {
        Ok(options) => options,
        Err(error) => {
            return finish(
                &cli.common.format,
                &cli.common.color,
                &command,
                display,
                Err(error),
            );
        }
    };
    let result = std::env::current_exe()
        .map_err(anyhow::Error::from)
        .and_then(|exe| wt_core::dispatch_with_worker(&command, &options, &exe));
    finish(
        &cli.common.format,
        &cli.common.color,
        &command,
        display,
        result,
    )
}

fn print_bundled_schema(name: &SchemaName) -> i32 {
    if matches!(name, SchemaName::Rule | SchemaName::Submission) {
        let schema = wt_core::package_schema(schema_string(name), 1);
        return match schema {
            Ok(schema) => {
                print_json(&schema);
                0
            }
            Err(error) => {
                eprintln!("{error}");
                2
            }
        };
    }
    let schema = match name {
        SchemaName::Rule => include_str!("../../../schemas/rule.schema.json"),
        SchemaName::Submission => include_str!("../../../schemas/submission.schema.json"),
        SchemaName::Tests => include_str!("../../../schemas/tests.schema.json"),
        SchemaName::Config => include_str!("../../../schemas/config.schema.json"),
        SchemaName::Result => include_str!("../../../schemas/result.schema.json"),
        SchemaName::Plan => include_str!("../../../schemas/plan.schema.json"),
        SchemaName::Review => include_str!("../../../schemas/review.schema.json"),
        SchemaName::Capabilities => include_str!("../../../schemas/capabilities.schema.json"),
    };
    print!("{schema}");
    if !schema.ends_with('\n') {
        println!();
    }
    0
}

fn options(cli: &Cli) -> anyhow::Result<Value> {
    let mut object = common_options(&cli.common);
    match &cli.command {
        Command::Capabilities => {}
        Command::Guide { topic } => {
            if let Some(topic) = topic {
                object.insert("topic".to_owned(), json!(topic));
            }
        }
        Command::Fmt { id, check, global } => {
            if let Some(id) = id {
                object.insert("id".to_owned(), json!(id));
            }
            insert_if_true(&mut object, "check", *check);
            insert_if_true(&mut object, "global", *global);
        }
        Command::Review {
            finding_id,
            decision,
            expect_evidence,
            expect_hash,
            reason_file,
            watch,
            rules,
            no_global,
            no_host_ignores,
        } => {
            object.insert("finding_id".to_owned(), json!(finding_id));
            object.insert(
                "decision".to_owned(),
                json!(match decision {
                    ReviewDecision::NeedsReview => "needs_review",
                    ReviewDecision::Acceptable => "acceptable",
                    ReviewDecision::ConfirmedIssue => "confirmed_issue",
                    ReviewDecision::AcceptedRisk => "accepted_risk",
                }),
            );
            object.insert("expect_evidence".to_owned(), json!(expect_evidence));
            if let Some(hash) = expect_hash {
                object.insert("expect_hash".to_owned(), json!(hash));
            }
            let mut rationale = String::new();
            File::open(reason_file)?
                .take(128 * 1024 + 1)
                .read_to_string(&mut rationale)?;
            if rationale.len() > 128 * 1024 {
                anyhow::bail!("rationale exceeds 128 KiB");
            }
            object.insert("rationale".to_owned(), json!(rationale));
            insert_if_nonempty(&mut object, "watch", watch);
            insert_if_nonempty(&mut object, "rules", rules);
            insert_if_true(&mut object, "no_global", *no_global);
            insert_if_true(&mut object, "no_host_ignores", *no_host_ignores);
        }
        Command::Reviews { finding_id } => {
            if let Some(id) = finding_id {
                object.insert("finding_id".to_owned(), json!(id));
            }
        }
        Command::Inspect {
            finding_id,
            no_global,
            no_host_ignores,
        } => {
            object.insert("finding_id".to_owned(), json!(finding_id));
            insert_if_true(&mut object, "no_global", *no_global);
            insert_if_true(&mut object, "no_host_ignores", *no_host_ignores);
        }
        Command::Init { global } => {
            object.insert("global".to_owned(), json!(global));
        }
        Command::New {
            json: positional,
            stdin,
            file,
            global,
        } => {
            object.insert("global".to_owned(), json!(global));
            object.insert(
                "submission".to_owned(),
                read_submission(positional.as_deref(), *stdin, file.as_ref())?,
            );
        }
        Command::Check {
            detail,
            submission,
            paths,
            include_ignored,
            no_host_ignores,
            no_global,
            rules,
            strict,
            no_cache,
            optimizer,
            changed,
            base,
            jobs,
            max_file_bytes,
            show_reviewed,
            allow_empty,
            stats,
        } => {
            object.insert(
                "detail".to_owned(),
                json!(match detail {
                    Detail::Summary => "summary",
                    Detail::Full => "full",
                }),
            );
            if let Some(path) = submission {
                object.insert("submission".to_owned(), read_json_file(path)?);
            }
            insert_if_true(&mut object, "include_ignored", *include_ignored);
            insert_if_true(&mut object, "no_host_ignores", *no_host_ignores);
            insert_if_true(&mut object, "no_global", *no_global);
            insert_if_nonempty(&mut object, "rules", rules);
            insert_if_true(&mut object, "strict", *strict);
            insert_if_true(&mut object, "no_cache", *no_cache);
            if let Some(mode) = optimizer {
                object.insert(
                    "optimizer".to_owned(),
                    json!(match mode {
                        Optimizer::Auto => "auto",
                        Optimizer::Off => "off",
                    }),
                );
            }
            insert_if_true(&mut object, "changed", *changed);
            if let Some(base) = base {
                object.insert("base".to_owned(), json!(base));
            }
            if let Some(jobs) = jobs {
                object.insert("jobs".to_owned(), json!(jobs));
            }
            if let Some(max_file_bytes) = max_file_bytes {
                object.insert("max_file_bytes".to_owned(), json!(max_file_bytes));
            }
            insert_if_true(&mut object, "show_reviewed", *show_reviewed);
            if matches!(cli.common.format, OutputFormat::Text) && *show_reviewed {
                object.insert("detail".to_owned(), json!("full"));
            }
            insert_if_true(&mut object, "allow_empty", *allow_empty);
            insert_if_true(&mut object, "stats", *stats);
            if !paths.is_empty() {
                object.insert("paths".to_owned(), json!(paths));
            }
        }
        Command::Stats {
            paths,
            include_ignored,
            no_host_ignores,
            no_global,
            rules,
            no_cache,
            optimizer,
            changed,
            base,
            jobs,
            max_file_bytes,
        } => {
            insert_if_true(&mut object, "include_ignored", *include_ignored);
            insert_if_true(&mut object, "no_host_ignores", *no_host_ignores);
            insert_if_true(&mut object, "no_global", *no_global);
            insert_if_nonempty(&mut object, "rules", rules);
            insert_if_true(&mut object, "no_cache", *no_cache);
            if let Some(mode) = optimizer {
                object.insert(
                    "optimizer".to_owned(),
                    json!(match mode {
                        Optimizer::Auto => "auto",
                        Optimizer::Off => "off",
                    }),
                );
            }
            insert_if_true(&mut object, "changed", *changed);
            if let Some(base) = base {
                object.insert("base".to_owned(), json!(base));
            }
            if let Some(jobs) = jobs {
                object.insert("jobs".to_owned(), json!(jobs));
            }
            if let Some(max_file_bytes) = max_file_bytes {
                object.insert("max_file_bytes".to_owned(), json!(max_file_bytes));
            }
            if !paths.is_empty() {
                object.insert("paths".to_owned(), json!(paths));
            }
        }
        Command::Plan {
            rules,
            no_global,
            optimizer,
        } => {
            insert_if_nonempty(&mut object, "rules", rules);
            insert_if_true(&mut object, "no_global", *no_global);
            if let Some(mode) = optimizer {
                object.insert(
                    "optimizer".to_owned(),
                    json!(match mode {
                        Optimizer::Auto => "auto",
                        Optimizer::Off => "off",
                    }),
                );
            }
        }
        Command::List { no_global } => {
            insert_if_true(&mut object, "no_global", *no_global);
        }
        Command::Config {
            no_global,
            max_file_bytes,
        } => {
            insert_if_true(&mut object, "no_global", *no_global);
            if let Some(bytes) = max_file_bytes {
                object.insert("max_file_bytes".to_owned(), json!(bytes));
            }
        }
        Command::Show { id } => {
            object.insert("id".to_owned(), json!(id));
        }
        Command::Validate {
            id,
            file,
            no_global,
        } => {
            insert_if_true(&mut object, "no_global", *no_global);
            if let Some(file) = file {
                object.insert("submission".to_owned(), read_json_file(file)?);
            } else if let Some(id) = id {
                object.insert("id".to_owned(), json!(id));
            }
        }
        Command::Test { id, no_global } => {
            insert_if_true(&mut object, "no_global", *no_global);
            if let Some(id) = id {
                object.insert("id".to_owned(), json!(id));
            }
        }
        Command::Update {
            id,
            stdin: _,
            expect_hash,
            allow_test_removal,
            reason,
            preview,
        } => {
            object.insert("id".to_owned(), json!(id));
            object.insert("expect_hash".to_owned(), json!(expect_hash));
            object.insert("submission".to_owned(), read_submission(None, true, None)?);
            insert_if_true(&mut object, "allow_test_removal", *allow_test_removal);
            if let Some(reason) = reason {
                object.insert("reason".to_owned(), json!(reason));
            }
            insert_if_true(&mut object, "preview", *preview);
        }
        Command::SetMode {
            id,
            mode,
            reason,
            no_global,
        } => {
            object.insert("id".to_owned(), json!(id));
            object.insert("mode".to_owned(), json!(mode_string(mode)));
            object.insert("reason".to_owned(), json!(reason));
            insert_if_true(&mut object, "no_global", *no_global);
        }
        Command::Explain {
            path,
            rules,
            no_global,
            include_ignored,
            no_host_ignores,
        } => {
            object.insert("paths".to_owned(), json!([path]));
            insert_if_nonempty(&mut object, "rules", rules);
            insert_if_true(&mut object, "no_global", *no_global);
            insert_if_true(&mut object, "include_ignored", *include_ignored);
            insert_if_true(&mut object, "no_host_ignores", *no_host_ignores);
        }
        Command::Schema { name } => {
            object.insert("schema".to_owned(), json!(schema_string(name)));
        }
        Command::Cache { command } => match command {
            CacheCommand::Clear => {}
        },
    }
    Ok(Value::Object(object))
}

fn common_options(common: &Common) -> Map<String, Value> {
    let mut object = Map::new();
    if let Some(root) = &common.root {
        object.insert("root".to_owned(), json!(root));
    }
    if let Some(global_dir) = &common.global_dir {
        object.insert("global_dir".to_owned(), json!(global_dir));
    }
    object.insert("format".to_owned(), json!(format_string(&common.format)));
    object.insert("color".to_owned(), json!(color_string(&common.color)));
    object
}

fn read_submission(
    positional: Option<&str>,
    stdin: bool,
    file: Option<&PathBuf>,
) -> anyhow::Result<Value> {
    match (positional, stdin, file) {
        (Some(value), false, None) => parse_bounded(value.as_bytes()),
        (None, true, None) => {
            let mut input = io::stdin().lock();
            parse_reader(&mut input)
        }
        (None, false, Some(path)) => {
            let mut input = File::open(path)?;
            parse_reader(&mut input)
        }
        _ => Err(anyhow::anyhow!(
            "exactly one submission source is required: JSON, --stdin, or --file PATH"
        )),
    }
}

fn read_json_file(path: &PathBuf) -> anyhow::Result<Value> {
    let mut input = File::open(path)?;
    parse_reader(&mut input)
}

fn parse_reader(reader: &mut dyn Read) -> anyhow::Result<Value> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_JSON_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_JSON_INPUT_BYTES {
        return Err(anyhow::anyhow!(
            "JSON input exceeds the 8 MiB CLI input limit"
        ));
    }
    parse_bounded(&bytes)
}

fn parse_bounded(bytes: &[u8]) -> anyhow::Result<Value> {
    if bytes.len() > MAX_JSON_INPUT_BYTES {
        return Err(anyhow::anyhow!(
            "JSON input exceeds the 8 MiB CLI input limit"
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|error| anyhow::anyhow!("JSON input is not UTF-8: {error}"))?;
    wt_core::parse_json(text).map_err(|error| anyhow::anyhow!("invalid strict JSON: {error}"))
}

fn finish(
    format: &OutputFormat,
    color: &ColorMode,
    command: &str,
    display: bool,
    result: anyhow::Result<Value>,
) -> i32 {
    let value = match result {
        Ok(value) => value,
        Err(error) => command_error(command, error.to_string()),
    };
    let exit_code = value
        .get("exit_code")
        .and_then(Value::as_i64)
        .map(|n| n as i32)
        .unwrap_or(if command == "capabilities" { 0 } else { 2 });
    match format {
        OutputFormat::Json => print_json(&value),
        OutputFormat::Text => print_text(&value, color, display),
    }
    exit_code
}

fn command_error(command: &str, error: String) -> Value {
    json!({
        "schema_version": 1,
        "command": command,
        "status": "error",
        "exit_code": 2,
        "detail": "summary",
        "data": {},
        "stages": [],
        "notices": [],
        "errors": [{"error":error}]
    })
}

fn print_json(value: &Value) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(error) => {
            eprintln!("unable to serialize JSON output: {error}");
        }
    }
}

/// Whether ANSI color is enabled for this invocation: `always`/`never`
/// force it; `auto` (the default) enables it only on a TTY, and a non-empty
/// `NO_COLOR` (https://no-color.org) disables it even then.
fn color_enabled(color: &ColorMode) -> bool {
    match color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            std::io::IsTerminal::is_terminal(&std::io::stdout())
                && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
        }
    }
}

fn print_text(value: &Value, color: &ColorMode, display: bool) {
    if let Some(error) = value
        .get("error")
        .or_else(|| value["errors"].get(0).and_then(|e| e.get("error")))
        .and_then(Value::as_str)
    {
        println!("error: {error}");
        return;
    }
    let color = color_enabled(color);
    match value.get("command").and_then(Value::as_str) {
        Some("guide") => {
            if let Some(text) = value["data"]["text"]
                .as_str()
                .or_else(|| value["text"].as_str())
            {
                println!("{text}");
            }
        }
        Some("stats") => print!("{}", text::render_stats(value, color)),
        Some("check") => print!("{}", text::render_check(value, color, display)),
        Some("reviews") => print!("{}", text::render_reviews(value)),
        Some("inspect") => print!("{}", text::render_inspect(value)),
        Some("review") => print!("{}", text::render_review(value)),
        _ => {
            if let Ok(text) = serde_json::to_string_pretty(value) {
                println!("{text}");
            }
        }
    }
}

fn insert_if_true(object: &mut Map<String, Value>, key: &str, value: bool) {
    if value {
        object.insert(key.to_owned(), json!(true));
    }
}

fn insert_if_nonempty(object: &mut Map<String, Value>, key: &str, values: &[String]) {
    if !values.is_empty() {
        object.insert(key.to_owned(), json!(values));
    }
}

fn command_string(command: &Command) -> String {
    match command {
        Command::Capabilities => "capabilities",
        Command::Guide { .. } => "guide",
        Command::Fmt { .. } => "fmt",
        Command::Review { .. } => "review",
        Command::Reviews { .. } => "reviews",
        Command::Inspect { .. } => "inspect",
        Command::Init { .. } => "init",
        Command::New { .. } => "new",
        Command::Check { .. } => "check",
        Command::Stats { .. } => "stats",
        Command::Plan { .. } => "plan",
        Command::List { .. } => "list",
        Command::Show { .. } => "show",
        Command::Validate { .. } => "validate",
        Command::Test { .. } => "test",
        Command::Update { .. } => "update",
        Command::SetMode { .. } => "set-mode",
        Command::Explain { .. } => "explain",
        Command::Config { .. } => "config",
        Command::Schema { .. } => "schema",
        Command::Cache { .. } => "cache-clear",
    }
    .to_owned()
}

fn command_name(args: &[OsString]) -> String {
    args.iter()
        .skip(1)
        .filter_map(|arg| arg.to_str())
        .find(|arg| {
            matches!(
                *arg,
                "capabilities"
                    | "guide"
                    | "fmt"
                    | "review"
                    | "reviews"
                    | "inspect"
                    | "init"
                    | "new"
                    | "check"
                    | "stats"
                    | "plan"
                    | "list"
                    | "show"
                    | "validate"
                    | "test"
                    | "update"
                    | "set-mode"
                    | "explain"
                    | "config"
                    | "schema"
                    | "cache"
            )
        })
        .unwrap_or("command")
        .to_owned()
}

fn json_hint(args: &[OsString]) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        arg == "--format=json"
            || (arg == "--format"
                && args.get(index + 1).and_then(|value| value.to_str()) == Some("json"))
    })
}

fn format_string(value: &OutputFormat) -> &'static str {
    match value {
        OutputFormat::Text => "text",
        OutputFormat::Json => "json",
    }
}

fn color_string(value: &ColorMode) -> &'static str {
    match value {
        ColorMode::Auto => "auto",
        ColorMode::Always => "always",
        ColorMode::Never => "never",
    }
}

fn mode_string(value: &Mode) -> &'static str {
    match value {
        Mode::Advisory => "advisory",
        Mode::Enforced => "enforced",
        Mode::Disabled => "disabled",
    }
}

fn schema_string(value: &SchemaName) -> &'static str {
    match value {
        SchemaName::Rule => "rule",
        SchemaName::Submission => "submission",
        SchemaName::Tests => "tests",
        SchemaName::Config => "config",
        SchemaName::Result => "result",
        SchemaName::Plan => "plan",
        SchemaName::Review => "review",
        SchemaName::Capabilities => "capabilities",
    }
}

pub fn print_help() {
    let _ = Cli::command().print_help();
}

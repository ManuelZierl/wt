//! Human-facing (`--format text`) rendering for `check`, `review`,
//! `reviews`, and `inspect`. `--format json` output is untouched by any of
//! this; it always carries the same fields this module reads, plus more.
//!
//! `check`'s layout borrows rustc's diagnostic shape: a one-line
//! `label[rule_id]: message` header, a `--> path:line:col` pointer, a
//! numbered source excerpt with a caret under the matched span, and `= key:
//! value` footer notes. Every full hash (finding id, evidence digest,
//! review-record digest) is shown as the shortest hex prefix that stays
//! unique for this run (see `wt-core`'s `idents` module) — full ids remain
//! valid input everywhere they were before.

use serde_json::Value;

const RED: &str = "31";
const YELLOW: &str = "33";
const DIM: &str = "2";

fn style(text: &str, code: &str, color: bool) -> String {
    if color {
        format!("\u{1b}[{code}m{text}\u{1b}[0m")
    } else {
        text.to_owned()
    }
}

fn severity_label(blocking: bool, color: bool) -> String {
    if blocking {
        style("error", RED, color)
    } else {
        style("warning", YELLOW, color)
    }
}

fn status_word(status: &str) -> &'static str {
    match status {
        "confirmed_issue" => "confirmed issue",
        "violation" => "violation",
        "accepted" => "accepted",
        _ => "needs review",
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut kept = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    kept.push('\u{2026}');
    kept
}

/// The leading whitespace of `line` up to (1-based, exclusive) `upto_column`,
/// with every character other than a tab replaced by a space and tabs kept
/// as tabs, so the caret line lines up under the source line in a terminal
/// regardless of its tab width.
fn caret_padding(line: &str, upto_column: u64) -> String {
    line.chars()
        .take(upto_column.saturating_sub(1) as usize)
        .map(|ch| if ch == '\t' { '\t' } else { ' ' })
        .collect()
}

fn gutter_width(finding: &Value) -> usize {
    let snippet = finding.get("snippet").filter(|s| !s.is_null());
    let mut max_number = finding["start_line"].as_u64().unwrap_or(1);
    if let Some(snippet) = snippet {
        for key in ["before", "matched", "after"] {
            if let Some(number) = snippet[key]["number"].as_u64() {
                max_number = max_number.max(number);
            }
        }
    }
    max_number.to_string().len()
}

/// One rustc-style block for an actionable finding: header, source excerpt
/// (when the checked source was available), and footer notes. Returned
/// without a trailing newline; callers join blocks with a blank line.
fn render_finding(finding: &Value, color: bool) -> String {
    let blocking = finding["blocking"] == true;
    let width = gutter_width(finding);
    let path = finding["path"].as_str().unwrap_or("<unknown>");
    let start_line = finding["start_line"].as_u64().unwrap_or(0);
    let start_column = finding["start_column"].as_u64().unwrap_or(0);
    let mut lines = Vec::new();
    lines.push(format!(
        "{}[{}]: {}",
        severity_label(blocking, color),
        finding["rule_id"].as_str().unwrap_or("?"),
        finding["message"].as_str().unwrap_or("")
    ));
    lines.push(format!(
        "{}--> {}:{}:{}",
        " ".repeat(width),
        path,
        start_line,
        start_column
    ));
    if let Some(snippet) = finding.get("snippet").filter(|s| !s.is_null()) {
        let blank = format!("{} |", " ".repeat(width));
        lines.push(blank.clone());
        if let (Some(number), Some(text)) = (
            snippet["before"]["number"].as_u64(),
            snippet["before"]["text"].as_str(),
        ) {
            lines.push(format!("{number:>width$} | {text}"));
        }
        let matched_text = snippet["matched"]["text"].as_str().unwrap_or("");
        lines.push(format!("{start_line:>width$} | {matched_text}"));
        let caret_end = snippet["caret_end_column"]
            .as_u64()
            .unwrap_or(start_column + 1)
            .max(start_column + 1);
        let extra_lines = snippet["extra_lines"].as_u64().unwrap_or(0);
        let padding = caret_padding(matched_text, start_column);
        let carets = "^".repeat((caret_end - start_column) as usize);
        let extra_note = if extra_lines > 0 {
            format!(
                " (+{extra_lines} more line{})",
                if extra_lines == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        lines.push(format!(
            "{} | {padding}{carets}{extra_note}",
            " ".repeat(width)
        ));
        if let (Some(number), Some(text)) = (
            snippet["after"]["number"].as_u64(),
            snippet["after"]["text"].as_str(),
        ) {
            lines.push(format!("{number:>width$} | {text}"));
        }
        lines.push(blank);
    }
    let status = finding["status"].as_str().unwrap_or("needs_review");
    let note_indent = " ".repeat(width + 1);
    let note = |text: String| format!("{note_indent}{}", style(&text, DIM, color));
    if finding["review_state"]["validity"] == "stale" {
        let reasons = finding["review_state"]["reasons"]
            .as_array()
            .map(|reasons| {
                reasons
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        lines.push(note(format!(
            "= status: needs review (reopened: {reasons})"
        )));
    } else {
        lines.push(note(format!("= status: {}", status_word(status))));
    }
    if let Some(help) = finding["help"].as_str() {
        lines.push(note(format!("= help: {help}")));
    }
    if let Some(evidence) = finding["evidence_prefix"].as_str() {
        lines.push(note(format!("= evidence: {evidence}")));
    }
    if let Some(prefix) = finding["finding_prefix"].as_str() {
        lines.push(note(format!("= inspect: wt inspect {prefix}")));
    }
    lines.join("\n")
}

/// A one-line summary for a finding already covered by a current
/// `confirmed_issue` decision (`Known issues`) or a current
/// `acceptable`/`accepted_risk` one (`Accepted`, shown only with
/// `--show-reviewed`). Kept visible, but out of the way of new findings.
fn render_compact(finding: &Value, color: bool) -> String {
    let blocking = finding["blocking"] == true;
    format!(
        "  {}[{}] {}:{}:{} {} {}",
        severity_label(blocking, color),
        finding["rule_id"].as_str().unwrap_or("?"),
        finding["path"].as_str().unwrap_or("<unknown>"),
        finding["start_line"].as_u64().unwrap_or(0),
        finding["start_column"].as_u64().unwrap_or(0),
        finding["finding_prefix"].as_str().unwrap_or(""),
        truncate(finding["message"].as_str().unwrap_or(""), 72)
    )
}

fn count_noun(count: u64, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn needs_review_phrase(count: u64) -> String {
    if count == 1 {
        "1 needs review".to_owned()
    } else {
        format!("{count} need review")
    }
}

fn summary_line(value: &Value) -> String {
    let summary = &value["summary"];
    let actionable = summary["actionable_findings"].as_u64().unwrap_or(0);
    let known = summary["known_issues"].as_u64().unwrap_or(0);
    let need_review = actionable.saturating_sub(known);
    let blocking = summary["blocking_diagnostics"].as_u64().unwrap_or(0);
    let accepted = summary["reviewed_findings"].as_u64().unwrap_or(0);
    let files = summary["checked_files"].as_u64().unwrap_or(0);
    let elapsed_seconds = summary["elapsed_ms"].as_u64().unwrap_or(0) as f64 / 1000.0;
    format!(
        "{} \u{b7} {} \u{b7} {blocking} blocking \u{b7} {accepted} accepted \u{b7} {} in {elapsed_seconds:.1}s",
        count_noun(known, "known issue", "known issues"),
        needs_review_phrase(need_review),
        count_noun(files, "file", "files"),
    )
}

/// Render `wt check`'s text output. `color` is whether ANSI styling is
/// enabled (already resolved from `--color`/`NO_COLOR`/TTY detection).
/// `show_reviewed` mirrors the flag of the same name: it reveals a
/// compact `Accepted` section for findings a current `acceptable`/
/// `accepted_risk` decision already hid from `diagnostics`.
pub fn render_check(value: &Value, color: bool, show_reviewed: bool) -> String {
    let mut blocks = Vec::new();
    if value["complete"] != true {
        blocks.push(format!(
            "{}: analysis incomplete",
            style("error", RED, color)
        ));
    }
    let errors = value["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|error| format!("analysis error: {error}"))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        blocks.push(errors.join("\n"));
    }
    let diagnostics = value["diagnostics"].as_array().cloned().unwrap_or_default();
    let (known, actionable): (Vec<Value>, Vec<Value>) = diagnostics
        .into_iter()
        .partition(|finding| finding["status"] == "confirmed_issue");
    for finding in &actionable {
        blocks.push(render_finding(finding, color));
    }
    if !known.is_empty() {
        let mut lines = vec!["Known issues:".to_owned()];
        lines.extend(known.iter().map(|finding| render_compact(finding, color)));
        blocks.push(lines.join("\n"));
    }
    if show_reviewed {
        let reviewed = value["reviewed"].as_array().cloned().unwrap_or_default();
        if !reviewed.is_empty() {
            let mut lines = vec!["Accepted:".to_owned()];
            lines.extend(
                reviewed
                    .iter()
                    .map(|finding| render_compact(finding, color)),
            );
            blocks.push(lines.join("\n"));
        }
    }
    blocks.push(summary_line(value));
    blocks.join("\n\n") + "\n"
}

fn stats_text(data: &Value, color: bool) -> String {
    let mut lines = Vec::new();
    for rule in data["rules"].as_array().into_iter().flatten() {
        let signal = rule["signal"].as_str().unwrap_or("unknown");
        let label = style_signal(signal, color);
        let decisions = &rule["review_decisions"];
        lines.push(format!(
            "{:<9} {} [{}, {}]",
            label,
            rule["id"].as_str().unwrap_or("<unknown>"),
            rule["mode"].as_str().unwrap_or("?"),
            rule["severity"].as_str().unwrap_or("?"),
        ));
        lines.push(format!(
            "  raw={} acceptable={} confirmed_issue={} accepted_risk={} needs_review={} stale={} scoped_files={}",
            rule["raw_findings"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".to_owned()),
            decisions["acceptable"].as_u64().unwrap_or(0),
            decisions["confirmed_issue"].as_u64().unwrap_or(0),
            decisions["accepted_risk"].as_u64().unwrap_or(0),
            decisions["needs_review"].as_u64().unwrap_or(0),
            rule["stale_decisions"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".to_owned()),
            rule["scoped_files"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".to_owned()),
        ));
        let unmatched = rule["unmatched_include"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if !unmatched.is_empty() {
            lines.push(format!(
                "  unmatched include glob(s): {}",
                unmatched.join(", ")
            ));
        }
    }
    let signals = &data["signals"];
    lines.push(format!(
        "{} useful, {} watch, {} quiet, {} noisy, {} active, {} dead, {} disabled, {} unknown; {}.",
        signals["useful"].as_u64().unwrap_or(0),
        signals["watch"].as_u64().unwrap_or(0),
        signals["quiet"].as_u64().unwrap_or(0),
        signals["noisy"].as_u64().unwrap_or(0),
        signals["active"].as_u64().unwrap_or(0),
        signals["dead"].as_u64().unwrap_or(0),
        signals["disabled"].as_u64().unwrap_or(0),
        signals["unknown"].as_u64().unwrap_or(0),
        if data["complete"] == true {
            "analysis complete"
        } else {
            "analysis incomplete"
        }
    ));
    lines.join("\n") + "\n"
}

pub fn render_stats(value: &Value, color: bool) -> String {
    stats_text(&value["data"], color)
}

fn style_signal(signal: &str, color: bool) -> String {
    let label = signal.to_uppercase();
    let code = match signal {
        "dead" | "noisy" => RED,
        "unknown" => YELLOW,
        "useful" | "watch" | "quiet" => "32",
        _ => "0",
    };
    style(&label, code, color)
}

/// `wt reviews`: one compact line per stored decision, prefixed hashes
/// throughout. `wt reviews FINDING_ID` (already resolved to exactly one
/// record by `wt-core`) instead shows that one record's full rationale.
pub fn render_reviews(value: &Value) -> String {
    let reviews = value["data"]["reviews"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if reviews.is_empty() {
        return "no stored review decisions.\n".to_owned();
    }
    if reviews.len() == 1 {
        let entry = &reviews[0];
        let record = &entry["record"];
        let mut lines = vec![
            format!(
                "{} {} (revision {}, record {})",
                record["decision"].as_str().unwrap_or("?"),
                entry["finding_prefix"].as_str().unwrap_or(""),
                entry["revision"],
                entry["record_digest_prefix"].as_str().unwrap_or("")
            ),
            format!("rule: {}", record["rule_id"].as_str().unwrap_or("?")),
            format!("path: {}", record["path"].as_str().unwrap_or("?")),
            format!(
                "evidence: {}",
                entry["evidence_prefix"].as_str().unwrap_or("")
            ),
        ];
        if let Some(rationale) = entry["rationale"].as_str() {
            lines.push(String::new());
            lines.push(rationale.trim_end().to_owned());
        }
        return lines.join("\n") + "\n";
    }
    let mut lines = Vec::new();
    for entry in &reviews {
        let record = &entry["record"];
        lines.push(format!(
            "{:<15} {} rev {}  {}  {}",
            record["decision"].as_str().unwrap_or("?"),
            entry["finding_prefix"].as_str().unwrap_or(""),
            entry["revision"],
            record["rule_id"].as_str().unwrap_or("?"),
            record["path"].as_str().unwrap_or("?"),
        ));
    }
    lines.join("\n") + "\n"
}

/// `wt inspect`: current observation plus, when one exists, the stored
/// decision it's judged against.
pub fn render_inspect(value: &Value) -> String {
    let data = &value["data"];
    let mut lines = vec![format!(
        "finding {} ({})",
        data["finding_prefix"].as_str().unwrap_or(""),
        data["observation"].as_str().unwrap_or("?")
    )];
    let finding = &data["finding"];
    if !finding.is_null() {
        lines.push(format!(
            "rule: {}",
            finding["rule_id"].as_str().unwrap_or("?")
        ));
        lines.push(format!(
            "path: {}:{}:{}",
            finding["path"].as_str().unwrap_or("?"),
            finding["start_line"],
            finding["start_column"]
        ));
        lines.push(format!(
            "status: {}",
            status_word(finding["status"].as_str().unwrap_or("needs_review"))
        ));
        if let Some(message) = finding["message"].as_str() {
            lines.push(format!("message: {message}"));
        }
    }
    let reasons = data["stale_reasons"]
        .as_array()
        .filter(|reasons| !reasons.is_empty())
        .map(|reasons| {
            reasons
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        });
    if let Some(reasons) = reasons {
        lines.push(format!("reopened because: {reasons}"));
    }
    let previous = &data["previous"];
    if !previous.is_null() {
        let record = &previous["record"];
        lines.push(format!(
            "previous decision: {} (revision {}, record {})",
            record["decision"].as_str().unwrap_or("?"),
            previous["revision"],
            previous["record_digest_prefix"].as_str().unwrap_or("")
        ));
    }
    lines.push(format!(
        "source diff: {}",
        if data["source_diff"].is_null() {
            "unavailable"
        } else {
            "available (see --format json)"
        }
    ));
    lines.join("\n") + "\n"
}

/// `wt review`: confirmation of the decision just written.
pub fn render_review(value: &Value) -> String {
    let data = &value["data"];
    format!(
        "recorded {} for {} (revision {}, record {})\npath: {}\n",
        data["decision"].as_str().unwrap_or("?"),
        data["finding_prefix"].as_str().unwrap_or(""),
        data["revision"],
        data["record_prefix"].as_str().unwrap_or(""),
        data["path"].as_str().unwrap_or("?"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_finding() -> Value {
        json!({
            "rule_id": "local/current-cwd-label",
            "path": "AGENTS.md",
            "message": "label cwd Current shell directory only at ...",
            "help": "Verify the branch uses ...",
            "blocking": false,
            "status": "needs_review",
            "start_line": 377, "start_column": 44,
            "end_line": 377, "end_column": 66,
            "finding_prefix": "aa1af802", "evidence_prefix": "bb2c1904",
            "review_state": {"validity": "unreviewed"},
            "snippet": {
                "before": {"number": 376, "text": "    let previous = 1;"},
                "matched": {"number": 377, "text": "    label cwd Current shell directory only"},
                "after": {"number": 378, "text": "    let next = 2;"},
                "caret_end_column": 66, "extra_lines": 0
            }
        })
    }

    #[test]
    fn full_finding_uses_rustc_style_frame_with_caret_and_footer() {
        let rendered = render_finding(&base_finding(), false);
        let expected = "warning[local/current-cwd-label]: label cwd Current shell directory only at ...\n   --> AGENTS.md:377:44\n    |\n376 |     let previous = 1;\n377 |     label cwd Current shell directory only\n    |                                           ^^^^^^^^^^^^^^^^^^^^^^\n378 |     let next = 2;\n    |\n    = status: needs review\n    = help: Verify the branch uses ...\n    = evidence: bb2c1904\n    = inspect: wt inspect aa1af802";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn blocking_finding_is_labeled_error_not_warning() {
        let mut finding = base_finding();
        finding["blocking"] = json!(true);
        assert!(render_finding(&finding, false).starts_with("error[local/current-cwd-label]"));
    }

    #[test]
    fn stale_decision_reopens_to_needs_review_with_reasons() {
        let mut finding = base_finding();
        finding["status"] = json!("needs_review");
        finding["review_state"] = json!({"validity": "stale", "reasons": ["owner_file_changed"]});
        let rendered = render_finding(&finding, false);
        assert!(rendered.contains("= status: needs review (reopened: owner_file_changed)"));
    }

    #[test]
    fn confirmed_issue_status_word_reads_confirmed_issue() {
        let mut finding = base_finding();
        finding["status"] = json!("confirmed_issue");
        finding["review_state"] = json!({"validity": "current", "decision": "confirmed_issue"});
        assert!(render_finding(&finding, false).contains("= status: confirmed issue"));
    }

    #[test]
    fn violation_status_with_no_snippet_omits_the_frame() {
        let mut finding = base_finding();
        finding["status"] = json!("violation");
        finding["snippet"] = Value::Null;
        let rendered = render_finding(&finding, false);
        assert!(!rendered.contains(" | "));
        assert!(rendered.contains("--> AGENTS.md:377:44"));
        assert!(rendered.contains("= status: violation"));
    }

    #[test]
    fn snippet_at_the_first_line_omits_a_before_line() {
        let mut finding = base_finding();
        finding["start_line"] = json!(1);
        finding["snippet"]["before"] = Value::Null;
        finding["snippet"]["matched"]["number"] = json!(1);
        let rendered = render_finding(&finding, false);
        assert!(!rendered.contains("376 |"));
        assert!(rendered.contains("  1 |") || rendered.contains("1 |"));
    }

    #[test]
    fn snippet_at_the_last_line_omits_an_after_line() {
        let mut finding = base_finding();
        finding["snippet"]["after"] = Value::Null;
        let rendered = render_finding(&finding, false);
        assert!(!rendered.contains("378 |"));
    }

    #[test]
    fn tabs_before_the_match_are_preserved_in_the_caret_padding() {
        let mut finding = base_finding();
        finding["start_column"] = json!(2);
        finding["snippet"]["matched"]["text"] = json!("\tlabel");
        finding["snippet"]["caret_end_column"] = json!(7);
        let rendered = render_finding(&finding, false);
        let caret_line = rendered
            .lines()
            .find(|line| line.contains('^'))
            .expect("caret line present");
        assert!(caret_line.starts_with("    | \t^^^^^"));
    }

    #[test]
    fn multibyte_matched_text_is_measured_in_chars_not_bytes() {
        let mut finding = base_finding();
        finding["start_column"] = json!(1);
        finding["snippet"]["matched"]["text"] = json!("caf\u{e9} match");
        finding["snippet"]["caret_end_column"] = json!(5);
        let rendered = render_finding(&finding, false);
        assert!(rendered.contains("caf\u{e9} match"));
        let caret_line = rendered
            .lines()
            .find(|line| line.contains('^'))
            .expect("caret line present");
        assert_eq!(caret_line, "    | ^^^^");
    }

    #[test]
    fn multiline_match_carets_to_end_of_first_line_and_notes_the_rest() {
        let mut finding = base_finding();
        finding["end_line"] = json!(379);
        finding["snippet"]["matched"]["text"] = json!("    label cwd");
        finding["snippet"]["caret_end_column"] = json!(14);
        finding["snippet"]["extra_lines"] = json!(2);
        let rendered = render_finding(&finding, false);
        assert!(rendered.contains("(+2 more lines)"));
    }

    #[test]
    fn known_issue_is_rendered_compact_and_omits_the_frame() {
        let mut finding = base_finding();
        finding["status"] = json!("confirmed_issue");
        let value = json!({
            "complete": true, "errors": [],
            "diagnostics": [finding],
            "summary": {"actionable_findings": 1, "known_issues": 1, "blocking_diagnostics": 0,
                "reviewed_findings": 0, "checked_files": 3, "elapsed_ms": 250}
        });
        let rendered = render_check(&value, false, false);
        assert!(rendered.contains("Known issues:"));
        assert!(rendered.contains("aa1af802"));
        assert!(!rendered.contains("-->"));
        assert!(rendered.contains("1 known issue \u{b7} 0 need review \u{b7} 0 blocking \u{b7} 0 accepted \u{b7} 3 files in 0.2s"));
    }

    #[test]
    fn singular_counts_read_naturally() {
        let value = json!({
            "complete": true, "errors": [], "diagnostics": [],
            "summary": {"actionable_findings": 1, "known_issues": 0, "blocking_diagnostics": 1,
                "reviewed_findings": 1, "checked_files": 1, "elapsed_ms": 40}
        });
        assert_eq!(
            summary_line(&value),
            "0 known issues \u{b7} 1 needs review \u{b7} 1 blocking \u{b7} 1 accepted \u{b7} 1 file in 0.0s"
        );
    }

    #[test]
    fn incomplete_analysis_is_loudly_reported() {
        let value = json!({
            "complete": false, "errors": [{"error": "no_eligible_files"}], "diagnostics": [],
            "summary": {"actionable_findings": 0, "known_issues": 0, "blocking_diagnostics": 0,
                "reviewed_findings": 0, "checked_files": 0, "elapsed_ms": 0}
        });
        let rendered = render_check(&value, false, false);
        assert!(rendered.starts_with("error: analysis incomplete"));
        assert!(rendered.contains("analysis error:"));
    }
}

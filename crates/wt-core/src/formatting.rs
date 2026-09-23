//! Token-preserving WRL formatting. Parsing/contract validation remains owned
//! by wt-runtime; formatting never rewrites expressions or moves queries.
use anyhow::{bail, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Word,
    String,
    Comment,
    LineComment,
    Symbol,
}
#[derive(Clone, Debug)]
struct Token<'a> {
    text: &'a str,
    kind: Kind,
    starts_line: bool,
}

fn tokens(source: &str) -> Result<Vec<Token<'_>>> {
    if source.len() > 128 * 1024 {
        bail!("WRL source exceeds 128 KiB");
    }
    let mut result = Vec::new();
    let mut offset = 0;
    let mut starts_line = true;
    while offset < source.len() {
        let rest = &source[offset..];
        let c = rest.chars().next().unwrap();
        if c.is_whitespace() {
            starts_line |= c == '\n';
            offset += c.len_utf8();
            continue;
        }
        let start = offset;
        let kind;
        if rest.starts_with("//") {
            offset += rest.find('\n').unwrap_or(rest.len());
            kind = Kind::LineComment;
        } else if rest.starts_with("/*") {
            offset += 2;
            let mut depth = 1;
            while offset < source.len() && depth > 0 {
                if source[offset..].starts_with("/*") {
                    depth += 1;
                    offset += 2;
                } else if source[offset..].starts_with("*/") {
                    depth -= 1;
                    offset += 2;
                } else {
                    offset += source[offset..].chars().next().unwrap().len_utf8();
                }
            }
            if depth != 0 {
                bail!("unterminated WRL comment");
            }
            kind = Kind::Comment;
        } else if c == '"' {
            offset += 1;
            let mut closed = false;
            while offset < source.len() {
                let next = source[offset..].chars().next().unwrap();
                offset += next.len_utf8();
                if next == '\\' {
                    if let Some(escaped) = source[offset..].chars().next() {
                        offset += escaped.len_utf8();
                    }
                } else if next == '"' {
                    closed = true;
                    break;
                }
            }
            if !closed {
                bail!("unterminated WRL string");
            }
            kind = Kind::String;
        } else if c.is_alphanumeric() || c == '_' {
            offset += c.len_utf8();
            while let Some(next) = source[offset..].chars().next() {
                if !next.is_alphanumeric() && next != '_' {
                    break;
                }
                offset += next.len_utf8();
            }
            kind = Kind::Word;
        } else {
            let length = [
                "::", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "**", "<<",
                ">>", "..",
            ]
            .iter()
            .find(|op| rest.starts_with(**op))
            .map_or(c.len_utf8(), |op| op.len());
            offset += length;
            kind = Kind::Symbol;
        }
        result.push(Token {
            text: &source[start..offset],
            kind,
            starts_line,
        });
        starts_line = false;
    }
    Ok(result)
}

pub fn format_source(source: &str) -> Result<String> {
    let input = tokens(source)?;
    let mut output = String::new();
    let mut depth = 0usize;
    let mut line_start = true;
    for (index, token) in input.iter().enumerate() {
        let previous = index.checked_sub(1).map(|i| &input[i]);
        let next = input.get(index + 1);
        if token.text == "}" {
            depth = depth
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("unbalanced WRL braces"))?;
        }
        if token.text == "}"
            || (token.starts_line && matches!(token.kind, Kind::Comment | Kind::LineComment))
        {
            newline(&mut output, &mut line_start);
        }
        if line_start {
            output.push_str(&"    ".repeat(depth));
            line_start = false;
        } else {
            let tight_before = matches!(token.text, ")" | "," | ";" | "." | "::")
                || (token.text == "("
                    && previous
                        .is_some_and(|p| p.kind == Kind::Word && !matches!(p.text, "if" | "for")));
            let tight_after = previous.is_some_and(|p| matches!(p.text, "(" | "." | "::" | "!"));
            if !tight_before && !tight_after && !output.ends_with([' ', '\n']) {
                output.push(' ');
            }
        }
        output.push_str(token.text);
        match token.text {
            "{" => {
                depth += 1;
                newline(&mut output, &mut line_start);
            }
            "}" if next.is_some_and(|t| t.text == "else") => {
                output.push(' ');
            }
            "}" => newline(&mut output, &mut line_start),
            ";" if next.is_some_and(|t| {
                matches!(t.kind, Kind::Comment | Kind::LineComment) && !t.starts_line
            }) =>
            {
                output.push(' ');
            }
            ";" => newline(&mut output, &mut line_start),
            _ if token.kind == Kind::LineComment => newline(&mut output, &mut line_start),
            _ => {}
        }
    }
    if depth != 0 {
        bail!("unbalanced WRL braces");
    }
    newline(&mut output, &mut line_start);
    // Whitespace must not merge tokens, split operators, or modify comments.
    let after = tokens(&output)?;
    if input
        .iter()
        .map(|t| (&t.kind, t.text))
        .ne(after.iter().map(|t| (&t.kind, t.text)))
    {
        bail!("formatter could not preserve the WRL token stream");
    }
    if output.len() > 128 * 1024 {
        bail!("formatted WRL exceeds 128 KiB");
    }
    Ok(output)
}

fn newline(output: &mut String, line_start: &mut bool) {
    if !*line_start {
        output.push('\n');
    }
    *line_start = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readable_and_idempotent() {
        let source = "for m in rx::find_all(file, \"x\") { if m.text.contains(\"{ ; }\") { emit(m.span, \"hit\"); } else { continue; } }";
        let formatted = format_source(source).unwrap();
        assert!(formatted.contains("\n    if m.text.contains(\"{ ; }\") {\n"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
    #[test]
    fn preserves_comments_strings_and_short_circuit() {
        let source = "// explanation\nif !file.text.contains(\"x\\\";\") && true { /* outer /* inner */ end */ emit(file.span, \"hit\"); // keep\n}";
        let formatted = format_source(source).unwrap();
        assert!(formatted.contains("/* outer /* inner */ end */"));
        assert!(formatted.contains("// keep"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
    #[test]
    fn refuses_incomplete_source() {
        for source in ["if true {", "/* unterminated", "\"unterminated"] {
            assert!(format_source(source).is_err());
        }
    }
}

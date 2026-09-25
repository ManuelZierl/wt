use super::*;

fn query_manifest() -> JsonValue {
    json!({
        "execution": "file",
        "patterns": {"call": "request\\((?P<body>[^\\r\\n]*)\\)"},
        "diagnostics": {"hit": {"kind": "violation"}},
        "code": {"language": "wt-rule-1", "capabilities": ["text.v1", "ast.v1"]}
    })
}

#[test]
fn repeated_regex_consumers_hash_one_source_snapshot() {
    let program = compile(
        &query_manifest(),
        "for m in rx::find_all(file, \"call\") { emit(m.span, \"hit\"); }",
    )
    .unwrap();
    let files = [SourceFile {
        path: "source.txt".into(),
        text: format!("request({})", "x".repeat(64 * 1024)).into(),
    }];
    let mut arena = QueryArena::new(true);
    for _ in 0..100 {
        let result = program.execute(&files, &mut arena).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].end_byte, files[0].text.len());
    }
    assert_eq!(arena.stats()["regex_evaluations"], 1);
    assert_eq!(arena.stats()["shared_result_hits"], 99);
    assert_eq!(arena.stats()["residual_iterations"], 100);
    arena.clear_file_results();
    assert!(arena.stats()["peak_retained_bytes"].as_u64().unwrap() > 0);
    assert_eq!(
        arena.stats()["cache_key_bytes_hashed"],
        files[0].text.len() as u64
    );
}

#[test]
fn retained_peak_survives_text_scope_eviction() {
    let mut arena = QueryArena::new(true);
    let mut temporary = 0;
    let key = |path: &str, digest: u8| TextKey {
        path: path.to_owned(),
        digest: [digest; 32],
        operation: "text::trim".to_owned(),
        arguments: Vec::new(),
    };
    arena
        .text_query(key("first.txt", 1), None, 0, &mut temporary, || {
            Ok(Value::Text("x".repeat(1024).into()))
        })
        .unwrap();
    let retained = arena.stats()["retained_memory_bytes"].as_u64().unwrap();
    assert!(retained >= 1024);
    arena
        .text_query(key("second.txt", 2), None, 0, &mut temporary, || {
            Ok(Value::Text("y".into()))
        })
        .unwrap();
    assert!(arena.stats()["peak_retained_bytes"].as_u64().unwrap() >= retained);
}

#[test]
fn changed_same_path_and_length_cannot_reuse_stale_matches() {
    let program = compile(
        &query_manifest(),
        "for m in rx::find_all(file, \"call\") { emit(m.span, \"hit\"); }",
    )
    .unwrap();
    let mut files = [SourceFile {
        path: "source.txt".into(),
        text: "request(x)".into(),
    }];
    let mut arena = QueryArena::new(true);
    assert_eq!(program.execute(&files, &mut arena).unwrap().len(), 1);
    files[0].text = "xxxxxxxxxx".into();
    assert!(program.execute(&files, &mut arena).unwrap().is_empty());
    assert_eq!(arena.stats()["regex_evaluations"], 2);
    assert_eq!(arena.stats()["cache_key_bytes_hashed"], 20);
}

#[test]
fn cached_match_text_and_capture_maps_share_storage() {
    let patterns =
        parse_patterns(Some(&json!({"call":"request\\((?P<body>[^\\r\\n]*)\\)"}))).unwrap();
    let file = SourceFile {
        path: "source.txt".into(),
        text: "request(é)".into(),
    };
    let mut arena = QueryArena::new(true);
    let mut temporary = 0;
    let first = arena
        .find_all(
            0,
            &file,
            &patterns["call"],
            0,
            file.text.len(),
            false,
            "find_all",
            &mut temporary,
        )
        .unwrap();
    let second = arena
        .find_all(
            7,
            &file,
            &patterns["call"],
            0,
            file.text.len(),
            false,
            "find_all",
            &mut temporary,
        )
        .unwrap();
    assert!(first[0].text.shares_storage(&file.text));
    assert!(second[0].text.shares_storage(&first[0].text));
    assert!(Arc::ptr_eq(&first[0].groups, &second[0].groups));
    let capture = first[0].groups["body"].as_ref().unwrap();
    assert_eq!(capture, "é");
    assert!(capture.shares_storage(&file.text));
    assert_eq!(first[0].file, 0);
    assert_eq!(second[0].file, 7);
}

#[test]
fn source_text_queries_do_not_rehash_a_file_for_each_rule() {
    let program = compile(
        &query_manifest(),
        "if text::contains(file.text, \"needle\") { emit(file.span, \"hit\"); }",
    )
    .unwrap();
    let files = [SourceFile {
        path: "source.txt".into(),
        text: "x".repeat(64 * 1024).into(),
    }];
    let mut arena = QueryArena::new(true);
    for _ in 0..100 {
        assert!(program.execute(&files, &mut arena).unwrap().is_empty());
    }
    assert_eq!(arena.stats()["text_evaluations"], 1);
    // Changing Value's physical layout must not lower per-consumer limits.
    assert_eq!(
        arena.stats()["temporary_bytes"],
        100 * (64 * 1024 + 208 + 6 + 32)
    );
    let bytes = arena.stats()["cache_key_bytes_hashed"].as_u64().unwrap();
    assert!(
        bytes <= files[0].text.len() as u64 + 100 * 6,
        "hashed {bytes} bytes"
    );
}

#[test]
fn cached_ast_matches_share_payloads_and_remap_only_local_handles() {
    let file = SourceFile {
        path: "source.tsx".into(),
        text: "const view = <input type=\"number\" step=\"any\" />;".into(),
    };
    let pattern =
        ast_match::compile_pattern(AstLanguage::Tsx, "<input type=\"$T\" step=\"any\" />").unwrap();
    let mut arena = QueryArena::new(true);
    let mut temporary = 0;
    let first = arena
        .ast_match(
            0,
            &file,
            AstLanguage::Tsx,
            "<input type=\"$T\" step=\"any\" />",
            &pattern,
            0,
            file.text.len(),
            false,
            &mut temporary,
        )
        .unwrap();
    let second = arena
        .ast_match(
            9,
            &file,
            AstLanguage::Tsx,
            "<input type=\"$T\" step=\"any\" />",
            &pattern,
            0,
            file.text.len(),
            false,
            &mut temporary,
        )
        .unwrap();
    assert!(Arc::ptr_eq(&first[0].groups, &second[0].groups));
    assert!(Arc::ptr_eq(&first[0].node_spans, &second[0].node_spans));
    assert_eq!(first[0].file, 0);
    assert_eq!(second[0].file, 9);
    assert_eq!(arena.stats()["parser_evaluations"], 1);
    assert_eq!(
        arena.stats()["cache_key_bytes_hashed"],
        file.text.len() as u64
    );
}

#[test]
fn optimized_and_reference_runs_keep_findings_and_logical_budgets_equal() {
    let program = compile(&query_manifest(),
        "for m in rx::find_all(file, \"call\") { if text::contains(m.text, \"foo\") { emit(m.span, \"hit\"); } }").unwrap();
    let files = [SourceFile {
        path: "source.txt".into(),
        text: "request(foo123)\r\nrequest(bar)".into(),
    }];
    let mut optimized = QueryArena::new(true);
    let mut reference = QueryArena::new(false);
    for _ in 0..10 {
        assert_eq!(
            program.execute(&files, &mut optimized).unwrap(),
            program.execute(&files, &mut reference).unwrap()
        );
    }
    for key in [
        "logical_steps",
        "native_bytes",
        "temporary_bytes",
        "residual_invocations",
    ] {
        assert_eq!(optimized.stats()[key], reference.stats()[key], "{key}");
    }
}

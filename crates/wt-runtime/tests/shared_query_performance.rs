//! Isolated release-mode query-runtime benchmark. No wall-time assertions.
//! Compilation, source construction, disk I/O and worker IPC are excluded.
use serde_json::{json, Value};
use std::hint::black_box;
use std::time::Instant;
use wt_runtime::{compile, QueryArena, SourceFile};

#[test]
#[ignore = "run explicitly in release mode; reports measurements, not timing thresholds"]
fn shared_query_performance() {
    let consumers = 128usize;
    let file_count = 32usize;
    let repeats = 3usize;
    let workloads = [
        ("whole-file-text", "x".repeat(64 * 1024), "if text::contains(file.text, \"NEVER_PRESENT\") { emit(file.span, \"hit\"); }"),
        ("shared-regex", format!("request({})", "x".repeat(64 * 1024)), "for m in rx::find_all(file, \"call\") { if text::contains(m.text, \"NEVER_PRESENT\") { emit(m.span, \"hit\"); } }"),
        ("many-matches", format!("request({})\n", "x".repeat(240)).repeat(256), "for m in rx::find_all(file, \"call\") { if text::contains(m.group_text(\"body\"), \"NEVER_PRESENT\") { emit(m.span, \"hit\"); } }"),
    ];
    let mut report = Vec::<Value>::new();
    for (name, body, source) in workloads {
        let manifest = json!({
            "execution":"file", "patterns":{"call":"request\\((?P<body>[^\\r\\n]*)\\)"},
            "diagnostics":{"hit":{"kind":"violation"}},
            "code":{"language":"wt-rule-1","capabilities":["text.v1","regex.v1"]}
        });
        let programs = (0..consumers).map(|_| compile(&manifest, source).unwrap()).collect::<Vec<_>>();
        let mut elapsed = Vec::new();
        let mut last_stats = Value::Null;
        for _ in 0..repeats {
            // Fresh snapshots on every repeat; lazy content identities start cold.
            let files = (0..file_count).map(|index| SourceFile {
                path: format!("source-{index}.txt"), text: body.clone().into(),
            }).collect::<Vec<_>>();
            let mut arena = QueryArena::new(true);
            let started = Instant::now();
            for file in &files {
                for program in &programs {
                    let diagnostics = program.execute(std::slice::from_ref(file), &mut arena).unwrap();
                    assert!(black_box(diagnostics).is_empty());
                }
                arena.clear_file_results();
            }
            elapsed.push(started.elapsed().as_secs_f64());
            last_stats = arena.stats();
        }
        elapsed.sort_by(f64::total_cmp);
        report.push(json!({
            "workload": name, "files": file_count, "consumers": consumers,
            "source_bytes": body.len() * file_count, "repeats": repeats,
            "seconds": elapsed, "median_seconds": elapsed[repeats / 2],
            "stats": last_stats
        }));
    }
    let report = serde_json::to_string_pretty(&report).unwrap();
    println!("{report}");
    if let Ok(path) = std::env::var("WT_BENCH_REPORT") { std::fs::write(path, report).unwrap(); }
}

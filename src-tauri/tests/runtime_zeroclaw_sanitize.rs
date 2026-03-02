use clawpal::runtime::zeroclaw::sanitize::{sanitize_line, sanitize_output};

#[test]
fn sanitize_removes_ansi_and_runtime_info_lines() {
    let raw =
        "[2m2026-02-25T08:04:09.490132Z[0m [32m INFO[0m zeroclaw::config::schema\nFinal answer";
    let out = sanitize_output(raw);
    assert_eq!(out, "Final answer");
}

#[test]
fn sanitize_line_strips_ansi_escapes() {
    let line = "\x1b[32mHello world\x1b[0m";
    assert_eq!(sanitize_line(line), Some("Hello world".to_string()));
}

#[test]
fn sanitize_line_strips_literal_ansi_fragments() {
    let line = "[0m[32m Some text [0m";
    assert_eq!(sanitize_line(line), Some("Some text".to_string()));
}

#[test]
fn sanitize_line_suppresses_empty_lines() {
    assert_eq!(sanitize_line(""), None);
    assert_eq!(sanitize_line("   "), None);
    assert_eq!(sanitize_line("\t  \t"), None);
}

#[test]
fn sanitize_line_suppresses_ansi_only_lines() {
    assert_eq!(sanitize_line("\x1b[0m"), None);
    assert_eq!(sanitize_line("[0m[32m[0m"), None);
}

#[test]
fn sanitize_line_suppresses_zeroclaw_trace_lines() {
    let trace = "[2m2026-02-25T08:04:09.490132Z[0m [32m INFO[0m zeroclaw::config::schema";
    assert_eq!(sanitize_line(trace), None);
}

#[test]
fn sanitize_line_suppresses_zeroclaw_warn_lines() {
    let warn_line = "2026-03-01T10:00:00Z  WARN  zeroclaw::runtime something happened";
    assert_eq!(sanitize_line(warn_line), None);
}

#[test]
fn sanitize_line_passes_through_normal_text() {
    assert_eq!(
        sanitize_line("Hello, how can I help?"),
        Some("Hello, how can I help?".to_string())
    );
}

#[test]
fn sanitize_line_trims_whitespace() {
    assert_eq!(
        sanitize_line("  some padded text  "),
        Some("some padded text".to_string())
    );
}

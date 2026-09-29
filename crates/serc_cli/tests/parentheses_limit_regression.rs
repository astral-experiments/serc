#[allow(dead_code)]
mod common;

use std::path::Path;
use std::process::{Command, Output};

fn oracle(source: &Path) -> Output {
    common::python_3145_command()
        .args([
            "-c",
            "import sys\ntry: compile(open(sys.argv[1], 'rb').read(), sys.argv[1], 'exec')\nexcept SyntaxError as e: print(e.msg); sys.exit(1)",
        ])
        .arg(source)
        .output()
        .unwrap()
}

fn serc(source: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_serc"))
        .args(["compile", "--check", "--quiet"])
        .arg(source)
        .output()
        .unwrap()
}

fn deep_expression(n: usize) -> String {
    format!("{}1{}", "f(".repeat(n), ")".repeat(n))
}

#[test]
fn cpython_rejects_the_201st_opening_delimiter() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nesting.py");
    for n in [200, 201] {
        std::fs::write(&path, format!("x={}\n", deep_expression(n))).unwrap();
        let native = oracle(&path);
        let compiled = serc(&path);
        assert_eq!(native.status.success(), n == 200, "native: {native:?}");
        assert_eq!(compiled.status.success(), n == 200, "serc: {compiled:?}");
        if n == 201 {
            assert!(
                String::from_utf8_lossy(&native.stdout).contains("too many nested parentheses")
            );
            assert!(
                String::from_utf8_lossy(&compiled.stderr).contains("too many nested parentheses")
            );
        }
    }
    // The expression's opening brace counts toward the same tokenizer limit.
    std::fs::write(&path, format!("x=f\"{{{}}}\"\n", deep_expression(200))).unwrap();
    let native = oracle(&path);
    let compiled = serc(&path);
    assert!(String::from_utf8_lossy(&native.stdout).contains("too many nested parentheses"));
    assert!(String::from_utf8_lossy(&compiled.stderr).contains("too many nested parentheses"));
}

#[test]
fn earlier_lexer_and_indentation_errors_take_precedence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("priority.py");
    for (prefix, expected) in [
        ("'unterminated\n", "unterminated string literal"),
        ("if 1:\n pass\n  x\n", "unexpected indent"),
        ("x=3_\n", "invalid decimal literal"),
        ("x=1e+\n", "invalid decimal literal"),
        ("x=2andx\n", "invalid decimal literal"),
        (")\n", "unmatched ')'"),
    ] {
        std::fs::write(&path, format!("{prefix}x={}\n", deep_expression(201))).unwrap();
        let native = oracle(&path);
        let compiled = serc(&path);
        assert!(!native.status.success(), "native: {native:?}");
        assert!(!compiled.status.success(), "serc: {compiled:?}");
        assert!(
            String::from_utf8_lossy(&native.stdout).contains(expected),
            "native: {native:?}"
        );
        assert!(
            !String::from_utf8_lossy(&compiled.stderr).contains("too many nested parentheses"),
            "serc: {compiled:?}"
        );
    }
}

#[test]
fn tokenizer_limit_takes_precedence_over_earlier_grammar_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("priority.py");
    for prefix in ["x=\n", "`\n", "else:\n pass\n", "2else\n", "2ifx\n"] {
        std::fs::write(&path, format!("{prefix}y={}\n", deep_expression(201))).unwrap();
        let native = oracle(&path);
        let compiled = serc(&path);
        assert!(String::from_utf8_lossy(&native.stdout).contains("too many nested parentheses"));
        assert!(String::from_utf8_lossy(&compiled.stderr).contains("too many nested parentheses"));
    }
}

#[test]
fn delimiter_types_and_literal_contents_follow_the_same_limit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("delimiters.py");
    let cases = [
        // An opening bracket of each kind contributes to the same depth.
        format!("x={}1{}\n", "([{".repeat(67), "}])".repeat(67)),
        // Delimiters in comments, string contents, and escaped f-string braces do not.
        format!(
            "# {}\nx = {:?}\ny = f\"{}\"\nz = {}\n",
            "([{".repeat(201),
            "([{".repeat(201),
            "{{".repeat(201),
            deep_expression(200)
        ),
        // The format specification's replacement field adds another brace.
        format!("x=f\"{{1:{{{}}}}}\"\n", deep_expression(199)),
        format!("x=f\"{{1:{{{}}}}}\"\n", deep_expression(198)),
    ];
    for source in cases {
        std::fs::write(&path, &source).unwrap();
        let native = oracle(&path);
        let compiled = serc(&path);
        assert_eq!(
            compiled.status.success(),
            native.status.success(),
            "native: {native:?}; serc: {compiled:?}"
        );
        if String::from_utf8_lossy(&native.stdout).contains("too many nested parentheses") {
            assert!(
                String::from_utf8_lossy(&compiled.stderr).contains("too many nested parentheses"),
                "serc: {compiled:?}"
            );
        }
    }
}

#[test]
fn nesting_error_points_to_the_first_delimiter_over_the_limit() {
    let source = format!("x={}\n", deep_expression(201));
    let error = serc::compile(&source, "nesting.py").unwrap_err();
    assert!(error.to_string().contains("too many nested parentheses"));
    let range = error.range().unwrap();
    assert_eq!(
        &source[range.start().to_usize()..range.end().to_usize()],
        "("
    );
    assert_eq!(range.start().to_usize(), 2 + 2 * 200 + 1);
}

#[test]
fn nesting_limit_matches_each_supported_python_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("versions.py");
    for version in common::PYTHON_VERSIONS {
        for (n, fstring) in [(200, false), (201, false), (199, true), (200, true)] {
            let expression = deep_expression(n);
            let source = if fstring {
                format!("x=f\"{{{expression}}}\"\n")
            } else {
                format!("x={expression}\n")
            };
            std::fs::write(&path, &source).unwrap();
            let native = common::python_command(version)
                .args([
                    "-c",
                    "import sys; compile(open(sys.argv[1], 'rb').read(), sys.argv[1], 'exec')",
                ])
                .arg(&path)
                .output()
                .unwrap();
            let compiled = common::check(&path, version);
            assert_eq!(
                compiled.status.success(),
                native.status.success(),
                "Python {version}, depth {n}, f-string {fstring}: native {native:?}; serc {compiled:?}"
            );
            if !native.status.success() {
                assert!(
                    String::from_utf8_lossy(&native.stderr).contains("too many nested parentheses")
                );
                assert!(
                    String::from_utf8_lossy(&compiled.stderr)
                        .contains("too many nested parentheses")
                );
            }
        }
    }
}

#[test]
fn template_string_nesting_depends_on_the_target_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("template.py");
    for version in common::PYTHON_VERSIONS {
        for prefix in ["t", "T", "tr", "rT"] {
            for source in [
                format!("x={prefix}\"{{{}}}\"\n", deep_expression(200)),
                format!("x={prefix}\"literal\"\ny={}\n", deep_expression(201)),
            ] {
                std::fs::write(&path, &source).unwrap();
                let native = common::python_command(version)
                    .args([
                        "-c",
                        "import sys; compile(open(sys.argv[1], 'rb').read(), sys.argv[1], 'exec')",
                    ])
                    .arg(&path)
                    .output()
                    .unwrap();
                let compiled = common::check(&path, version);
                assert_eq!(compiled.status.success(), native.status.success());
                assert_eq!(
                    String::from_utf8_lossy(&compiled.stderr)
                        .contains("too many nested parentheses"),
                    String::from_utf8_lossy(&native.stderr).contains("too many nested parentheses"),
                    "Python {version}, prefix {prefix}: native {native:?}; serc {compiled:?}"
                );
            }
        }
    }
}

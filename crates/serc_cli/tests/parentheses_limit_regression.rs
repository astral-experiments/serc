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
#[ignore = "Serc currently accepts inputs rejected by CPython's nesting limit"]
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
#[ignore = "Preserves tokenizer-error precedence when implementing the nesting limit"]
fn earlier_lexer_and_indentation_errors_take_precedence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("priority.py");
    for (prefix, expected) in [
        ("'unterminated\n", "unterminated string literal"),
        ("if 1:\n pass\n  x\n", "unexpected indent"),
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
#[ignore = "CPython's later tokenizer error takes priority over an earlier grammar error"]
fn tokenizer_limit_takes_precedence_over_earlier_grammar_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("priority.py");
    std::fs::write(&path, format!("x=\ny={}\n", deep_expression(201))).unwrap();
    let native = oracle(&path);
    let compiled = serc(&path);
    assert!(String::from_utf8_lossy(&native.stdout).contains("too many nested parentheses"));
    assert!(String::from_utf8_lossy(&compiled.stderr).contains("too many nested parentheses"));
}

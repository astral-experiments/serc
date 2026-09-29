#[allow(dead_code)]
mod common;

use serc::{CompileError, compile};

#[test]
fn expression_traversal_preserves_results_and_syntax_errors() {
    for source in [
        "result = (left + right) * scale\n",
        "result = (1 + 2) * (3 - 4)\n",
        "result = [item for item in values if not (not item)]\n",
        "async def f(value, flag):\n    return value if flag else None\n",
    ] {
        common::assert_matches_cpython_marshal(source, "expression_traversal.py");
    }
    assert!(matches!(
        compile("result = left +\n", "malformed_expression.py"),
        Err(CompileError::Parse(_))
    ));
    assert!(matches!(
        compile("return left + right\n", "invalid_scope.py"),
        Err(CompileError::InvalidSyntax { .. })
    ));
}

#[test]
fn postponed_annotations_preserve_source_spellings_and_parameters() {
    let source = r#"from __future__ import annotations
value: "\ud800" | (0x1_0000_0000_0000_0000).real | Resource.Type

def f(a: "Forward" | T, /, b: T[0] = 1, *args: T, c: T | None = 2, **kwargs: T) -> T[0]:
    pass

type Alias = "Forward" | T
"#;
    common::assert_matches_cpython_marshal(source, "annotation_traversal.py");
    for (source, expected) in [
        ("def f(a: T, /, a): pass\n", "duplicate"),
        ("def f(__debug__: T): pass\n", "__debug__"),
        ("def f(a: T, b: (x := 1)): pass\n", "annotation"),
        ("def f(a: T = (yield 1)): pass\n", "yield"),
    ] {
        let error = compile(source, "invalid_function.py").unwrap_err();
        assert!(
            error.to_string().to_ascii_lowercase().contains(expected),
            "{error:?}"
        );
    }
}

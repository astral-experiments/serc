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

#[test]
fn private_name_normalization_preserves_nested_scope_bindings() {
    let source = r"class C:
    def build(self, values):
        __x = 1
        function = lambda __x: _C__x
        closures = [(lambda: _C__x) for __x in values]
        generator = (__x for _C__x in values)
        return function, closures, generator
";
    common::assert_matches_cpython_marshal(source, "private_scope_traversal.py");
}

#[test]
fn condition_traversal_preserves_branch_and_loop_lowering() {
    let source = r"def choose(value, flag, values):
    if not (not value):
        return 1 if flag else 2
    while value and flag:
        break
    return [item for item in values if not (not item) and flag]
";
    common::assert_matches_cpython_marshal(source, "condition_traversal.py");
}

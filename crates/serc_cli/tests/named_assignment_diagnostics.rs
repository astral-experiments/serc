mod common;

use serc::{CompileError, CompileOptions, compile_with_options};

#[test]
fn attribute_named_expression_diagnostic_matches_python() {
    let sources = [
        "value = (obj.attr := 0)\n",
        "(obj.attr.more := 0)\n",
        "(factory().attr := 0)\n",
        "(obj[0].attr := 0)\n",
        "(((obj.attr)) := 0)\n",
        "((obj).attr := 0)\n",
        "(obj . attr := 0)\n",
        "(obj[α + 1].β := 1)\n",
        "(obj.\n attr := 1)\n",
        "(obj.\r\n attr := 1)\r\n",
        "(1 .real := 0)\n",
        "f'{(obj.attr := 0)}'\n",
        "f'{(obj.attr := 0)=}'\n",
        "class C:\n value = (obj.attr := 0)\n",
        "def f():\n return (obj.attr := 0)\n",
    ];
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in sources {
            let diagnostic = common::diagnostics::syntax_diagnostic_with_bytes(version, source);
            let normalized = source.replace("\r\n", "\n");
            let expected = diagnostic.character_range(&normalized);
            diagnostic.assert_error(source, &options, expected);
        }
    }
}

#[test]
fn other_named_expression_errors_are_preserved() {
    for source in [
        "(obj[0] := 0)\n",
        "(obj.attr :=)\n",
        "(obj.attr := 0\n",
        "(obj.attr := 0)\nx = 'unterminated\n",
        "(obj.attr := 0)\n)\n",
        "@\n(obj.attr := 0)\n",
        "(obj.attr := (other.x := 3))\n",
    ] {
        let error =
            compile_with_options(source, "case.py", &CompileOptions::default()).unwrap_err();
        assert!(
            matches!(error, CompileError::Parse(_)),
            "{source:?}: {error}"
        );
    }
    for source in [
        "(obj := 0)\n",
        "f'{obj.attr := 0}'\n",
        "value = '(obj.attr := 0)'\n",
        "# (obj.attr := 0)\npass\n",
    ] {
        assert!(compile_with_options(source, "case.py", &CompileOptions::default()).is_ok());
    }
}

#[test]
fn deep_attribute_target_does_not_overflow_visitor_stack() {
    let source = format!("(obj{} := 0)\n", ".field".repeat(2000));
    let error = compile_with_options(&source, "case.py", &CompileOptions::default()).unwrap_err();
    assert!(
        error
            .to_string()
            .ends_with("cannot use assignment expressions with attribute"),
        "{error}"
    );
}

#[test]
fn unknown_decimal_parse_limit_takes_precedence() {
    let source = format!(
        "match value:\n    case _:\n        value = (obj.attr := 0)\n        integer = {}\n",
        "9".repeat(641)
    );
    let error = compile_with_options(&source, "case.py", &CompileOptions::default()).unwrap_err();
    assert!(
        matches!(
            error,
            CompileError::Unsupported(ref message) if message == "decimal integer parsing limit is not known"
        ),
        "{error}"
    );
}

mod common;

use serc::{CompileError, CompileOptions, compile_with_options};

#[test]
fn unknown_decimal_parse_limit_takes_precedence() {
    let source = format!(
        "match value:\n    case _:\n        def f():\n            yield from a, b\n        integer = {}\n",
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

#[test]
fn yield_from_tuple_diagnostics_match_native() {
    let deep = format!("def f():\n    yield from {}a, b\n", "+".repeat(300));
    let many_stars = format!("def f():\n    yield from {}\n", vec!["*a"; 1000].join(", "));
    let cases = [
        deep.as_str(),
        many_stars.as_str(),
        "def f():\n    yield from a, b\n",
        "def f():\n    yield from a,\n",
        "def f():\n    yield from *a, b\n",
        "def f():\n    yield from a, *b\n",
        "def f():\n    yield from *a, *b\n",
        "def f():\n    yield from (*a), b\n",
        "def f():\n    yield from ((*a)), b\n",
        "def f():\n    yield from (*a), *b\n",
        "def f():\n    yield from (*a,), b\n",
        "def f():\n    yield from (a), b\n",
        "def f():\n    yield from (a,b), c\n",
        "def f():\n    yield from foo(a,b), c\n",
        "def f():\n    yield from ((foo(a,b))), c\n",
        "def f():\n    yield from a if b else c, d\n",
        "def f():\n    yield from (lambda a,b:a), c\n",
        "def f():\n    yield from (x:=a), b\n",
        "def f():\n    yield from [x for x in a], b\n",
        "def f():\n    yield from f\"{a,b}\", c\n",
        "def f():\n    yield from (f\"{(a,b)}\", x), c\n",
        "def f():\n    yield from (a # comment\n      , b), c\n",
        "def f():\n    yield from α, β\n",
        "def f():\n    yield from a \\\n      , b\n",
        "def f():\n    yield from (a, b), c, d\n",
        "def f():\n    yield from a, (b, c)\n",
        "def f():\n    yield from a \\\n       ,\n",
    ];
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in cases {
            let diagnostic = common::diagnostics::syntax_diagnostic(version, source);
            let expected = diagnostic.character_range(source);
            diagnostic.assert_error(source, &options, expected);
        }
    }
}

#[test]
fn yield_from_tuple_does_not_override_other_parser_errors() {
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in [
            "def f():\n    raise a, b\n",
            "def f():\n    yield from a, b\n    )\n",
            "def f():\n    yield from a, b\n    x = 123abc\n",
            "def f():\n    yield from a, b\n    x = 'unterminated\n",
        ] {
            let error = compile_with_options(source, "case.py", &options).unwrap_err();
            assert!(
                matches!(error, CompileError::Parse(_)),
                "{version}: {source:?}: {error}"
            );
        }
        for source in [
            "def f():\n    yield from (a, b)\n",
            "def f():\n    yield a, b\n",
            "def f():\n    yield from a # , b\n",
        ] {
            compile_with_options(source, "case.py", &options)
                .unwrap_or_else(|error| panic!("{version}: {source:?}: {error}"));
        }
    }
}

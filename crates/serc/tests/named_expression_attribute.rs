use std::error::Error;

use ruff_python_parser::{ParseError, ParseErrorType};
use serc::{CompileError, CompileOptions, PythonVersion, compile_with_options};

#[test]
fn assignment_expression_attribute_diagnostic() {
    for python_version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        for (source, highlighted) in [
            ("value = (obj.attr := 0)", "obj.attr"),
            ("value = ((obj.attr) := 0)", "obj.attr"),
            ("value = (obj[0].attr := 0)", "obj[0].attr"),
            ("value = (factory().attr := 0)", "factory().attr"),
            ("value = f\"{(obj.attr := 0)}\"", "obj.attr"),
            ("f((obj.attr := 1))", "obj.attr"),
            ("[x for x in xs if (obj.attr := 1)]", "obj.attr"),
            ("(obj.attr := 1, object.item := 2)", "obj.attr"),
            ("@(obj.attr := 0)\ndef f(): pass", "obj.attr"),
            ("@obj.attr := 1\ndef f(): pass", "obj.attr"),
            ("if obj.attr := 1: pass", "obj.attr"),
            ("while obj.attr := 1: pass", "obj.attr"),
            ("[obj.attr := 1]", "obj.attr"),
            ("{obj.attr := 1}", "obj.attr"),
            ("obj[obj.attr := 1]", "obj.attr"),
            ("obj[obj.attr := 1, 2]", "obj.attr"),
            ("match obj.attr := 1, 2:\n case _: pass", "obj.attr"),
            ("(obj.attr := 1)", "obj.attr"),
        ] {
            let error = compile_with_options(source, "attribute.py", &options).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("cannot use assignment expressions with attribute"),
                "{python_version}: {source}: {error}"
            );
            assert_eq!(&source[error.range().unwrap()], highlighted);
            let original = error
                .source()
                .unwrap()
                .downcast_ref::<ParseError>()
                .unwrap();
            assert!(matches!(
                original.error,
                ParseErrorType::InvalidNamedAssignmentTarget
            ));
            assert_eq!(Some(original.location), error.range());
        }

        for source in [
            "value = (obj[0] := 0)",
            "value = ((a + b) := 0)",
            "value = (obj.attr := )",
            "f(obj.attr := 1)",
            "[x for x in xs if obj.attr := 1]",
            "f(k=obj.attr := 1)",
            "{0: obj.attr := 1}",
            "x = obj.attr := 1",
            "x = first, obj.attr := 1",
            "def f():\n return first, obj.attr := 1",
            "x: int = obj.attr := 1",
            "x += obj.attr := 1",
            "def f():\n return obj.attr := 1",
            "raise obj.attr := 1",
            "assert obj.attr := 1",
            "for x in obj.attr := 1: pass",
            "with obj.attr := 1: pass",
            "(obj.attr := (other[0] := 1))",
            "(obj.attr := ((1 + 2) := 1))",
            "f((obj.attr := 1), [)",
            "(obj.attr := 1)\n\"unterminated",
        ] {
            let error = compile_with_options(source, "other.py", &options).unwrap_err();
            assert!(matches!(error, CompileError::Parse(_)), "{source}: {error}");
        }
    }
}

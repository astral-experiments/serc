use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

use crate::common;

const FILENAME: &str = "surrogate_docstring.py";
const TARGETS: [PythonVersion; 4] = [
    PythonVersion::Py312,
    PythonVersion::Py313,
    PythonVersion::Py314,
    PythonVersion::Py315,
];

#[test]
fn surrogate_docstrings_match_native_version_and_scope_rejections() {
    for (literal, failure) in [
        (r#""\ud800""#, r"character '\ud800' in position 0"),
        (r#""\udfff""#, r"character '\udfff' in position 0"),
        (r#""\U0000d800""#, r"character '\ud800' in position 0"),
        (r#""é𐀀\ud800""#, r"character '\ud800' in position 2"),
        (
            r#"("prefix " "\ud800" "\udfff" " suffix")"#,
            "characters in position 7-8",
        ),
        (
            r#""\N{REPLACEMENT CHARACTER}\udfffX\ud800""#,
            r"character '\udfff' in position 1",
        ),
        (
            r#"(r"\ud800" "\udfff")"#,
            r"character '\udfff' in position 6",
        ),
        (
            r#""""  first\n    \ud800""""#,
            r"character '\ud800' in position 12",
        ),
    ] {
        let expected = format!("'utf-8' codec can't encode {failure}: surrogates not allowed");
        for wrapper in [
            "$DOC\n",
            "def f():\n    $DOC\n",
            "async def f():\n    $DOC\n",
            "class C:\n    $DOC\n",
            "if False:\n    def f():\n        $DOC\n",
            "def f():\n    return\n    class C:\n        $DOC\n",
            "def f[T]():\n    $DOC\n",
            "class C[T]:\n    $DOC\n",
        ] {
            let source = wrapper.replace("$DOC", literal);
            assert_eq!(native_error(&source), expected, "{source}");
            for python_version in TARGETS {
                let options = CompileOptions {
                    python_version,
                    ..CompileOptions::default()
                };
                for compiled in [
                    compile_with_options(&source, FILENAME, &options),
                    compile_bytes_with_options(source.as_bytes(), FILENAME, &options),
                ] {
                    if python_version == PythonVersion::Py312 {
                        assert!(compiled.is_ok(), "{python_version}: {source}: {compiled:?}");
                        continue;
                    }
                    let error = compiled.unwrap_err();
                    let CompileError::InvalidDocstring { message, range } = &error else {
                        panic!("unexpected {python_version} error for {source}: {error:?}");
                    };
                    assert_eq!(message, &expected, "{python_version}: {source}");
                    assert_eq!(error.to_string(), expected);
                    assert_eq!(error.range(), Some(*range));
                    assert_eq!(
                        &source[*range],
                        literal.trim_start_matches('(').trim_end_matches(')'),
                    );
                }
            }
        }
    }
}

#[test]
fn surrogate_docstring_validation_keeps_non_docstrings_and_replacement_characters() {
    let source = r#"r"\ud800"
import os
"\ud800"
if True:
    "\ud800"
def f():
    "�\ufffd\N{REPLACEMENT CHARACTER}\U00010000"
    "\ud800"
    return "\udfff"
async def g():
    "\\ud800"
    return "\ud800"
class C:
    "\ufffd" r"\ud800"
    "\ud800"
class F:
    f"\ud800"
class B:
    "\ud800" + ""
"#;
    for python_version in TARGETS {
        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        assert!(compile_with_options(source, FILENAME, &options).is_ok());
        assert!(compile_bytes_with_options(source.as_bytes(), FILENAME, &options).is_ok());
    }
    common::assert_matches_cpython_marshal(source, FILENAME);
}

#[test]
fn surrogate_docstrings_preserve_future_directive_error_priority() {
    for source in [
        "'\\ud800'\nfrom __future__ import braces\n",
        "'\\ud800'\nfrom __future__ import nonexistent\n",
        "'\\ud800'\n0\nfrom __future__ import annotations\n",
        "def f():\n    '\\ud800'\nfrom __future__ import annotations\n",
        "def f():\n    '\\ud800'\n    from __future__ import annotations\n",
    ] {
        assert_eq!(native_error_kind(source), "SyntaxError", "{source}");
        for python_version in &TARGETS[1..] {
            let options = CompileOptions {
                python_version: *python_version,
                ..CompileOptions::default()
            };
            let error = compile_with_options(source, FILENAME, &options).unwrap_err();
            assert!(
                matches!(error, CompileError::InvalidSyntax { .. }),
                "{error:?}"
            );
        }
    }
    for source in [
        "'\\ud800'\nfrom __future__ import annotations\nreturn\n",
        "'\\ud800'\nfrom .__future__ import nonexistent\n",
        "def f():\n    '\\ud800'\n    break\n",
        "'\\ud800'\nasync def f():\n    return 1\n    yield 2\n",
        "'\\ud800'\ntry: pass\nexcept* Exception:\n    return\n",
        "'\\ud800'\nasync def f():\n    yield from ()\n",
    ] {
        assert_eq!(native_error_kind(source), "UnicodeEncodeError", "{source}");
        for python_version in &TARGETS[1..] {
            let options = CompileOptions {
                python_version: *python_version,
                ..CompileOptions::default()
            };
            let error = compile_with_options(source, FILENAME, &options).unwrap_err();
            assert!(
                matches!(error, CompileError::InvalidDocstring { .. }),
                "{error:?}"
            );
        }
    }
}

#[test]
fn surrogate_docstrings_preserve_declaration_error_priority_across_scopes() {
    for source in [
        "'\\ud800'\nnonlocal value\n",
        "def f():\n    '\\ud800'\ndef g(value):\n    global value\n",
        "def f():\n    value = 1\n    global value\ndef g():\n    '\\ud800'\n",
        "def f():\n    '\\ud800'\n    value = 1\n    nonlocal value\n",
        "'\\ud800'\nreturn\ndef g(value):\n    global value\n",
    ] {
        assert_eq!(native_error_kind(source), "SyntaxError", "{source}");
        for python_version in &TARGETS[1..] {
            let options = CompileOptions {
                python_version: *python_version,
                ..CompileOptions::default()
            };
            let error = compile_with_options(source, FILENAME, &options).unwrap_err();
            assert!(
                matches!(error, CompileError::InvalidSyntax { .. }),
                "{error:?}"
            );
        }
    }
}

#[test]
fn surrogate_docstrings_preserve_modern_symbol_error_priority() {
    for source in [
        "'\\ud800'\n__debug__ = 1\n",
        "'\\ud800'\nreturn\n__debug__ = 1\n",
        "'\\ud800'\ndef f(value, value): pass\n",
        "'\\ud800'\nclass C[T, T]: pass\n",
    ] {
        assert_eq!(native_error_kind(source), "SyntaxError", "{source}");
        for python_version in &TARGETS[2..] {
            let options = CompileOptions {
                python_version: *python_version,
                ..CompileOptions::default()
            };
            let error = compile_with_options(source, FILENAME, &options).unwrap_err();
            assert!(
                matches!(error, CompileError::InvalidSyntax { .. }),
                "{error:?}"
            );
        }
    }
}

fn native_error_kind(source: &str) -> String {
    let result = common::python_3145_command()
        .args([
            "-c",
            "import sys\ntry: compile(sys.argv[1], sys.argv[2], 'exec')\nexcept Exception as error: print(type(error).__name__)\nelse: raise AssertionError('CPython accepted the docstring')",
            source,
            FILENAME,
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

fn native_error(source: &str) -> String {
    let result = common::python_3145_command()
        .args([
            "-c",
            "import sys\ntry: compile(sys.argv[1], sys.argv[2], 'exec')\nexcept UnicodeEncodeError as error: print(error)\nelse: raise AssertionError('CPython accepted the docstring')",
            source,
            FILENAME,
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

mod common;
mod compatibility_cases;

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::fmt::Write as _;

    use super::common::{
        assert_cpython_and_serc_have_pop_except_at, assert_executes,
        assert_matches_cpython_marshal, assert_matches_cpython_marshal_bytes,
        assert_matches_cpython_pop_except_positions,
    };
    use serc::{
        CPYTHON_TARGET, CompileError, compile, compile_bytes, compile_bytes_to_pyc, compile_to_pyc,
        decode_source,
    };
    use serc_fixtures as fixtures;

    #[test]
    fn exposes_the_pinned_cpython_source_identity() {
        assert_eq!(CPYTHON_TARGET.tag, "v3.14.5");
        assert_eq!(
            CPYTHON_TARGET.commit,
            "5607950ef232dad16d75c0cf53101d9649d89115"
        );
    }

    #[test]
    fn terminal_guard_returns_and_raises_match_cpython() {
        let source = "def guard(flag, other):\n    if flag:\n        return flag + 1\n    if other is None:\n        raise ValueError('missing', other)\n    return other\n\ndef side_local(flag):\n    if flag:\n        value = 4\n        return value\n    return value\n\ndef side_deletion(flag):\n    if flag:\n        del flag\n        raise RuntimeError('deleted')\n    return flag\n\ndef continuing_binding(flag, value):\n    prepared = value\n    if flag:\n        prepared = flag\n        return prepared\n    if prepared is None:\n        return 42\n    return prepared\n\ndef side_check(flag):\n    if flag:\n        result\n        raise RuntimeError\n    return result\n";
        assert_matches_cpython_marshal(source, "terminal_guard_paths.py");
    }

    #[test]
    fn short_circuit_and_exhaustive_terminal_paths_match_cpython() {
        let source = r"
def reraise():
    raise

def chained(exception, cause):
    message = str(exception)
    raise ValueError(message) from cause

def exhaustive(first, second):
    if first:
        return 1
    elif second:
        raise TypeError(second)
    else:
        return 3

def conjunction(first, second, third):
    if first and second and third:
        return first + third
    return second

def partial_assignment(first, second):
    if first and (local := second):
        return local
    return local

def continuing_assignment(first, second):
    if (local := first) and second:
        return second
    return local
";
        assert_matches_cpython_marshal(source, "short_circuit_terminal_paths.py");
    }

    #[test]
    fn emits_a_python_3145_pyc_header() {
        let pyc = compile_to_pyc("answer = 42\n", "answer.py", 123).unwrap();
        assert_eq!(&pyc[..4], &CPYTHON_TARGET.magic_number);
        assert_eq!(&pyc[4..8], &0_u32.to_le_bytes());
        assert_eq!(&pyc[8..12], &123_u32.to_le_bytes());
        assert_eq!(&pyc[12..16], &12_u32.to_le_bytes());
    }

    #[test]
    fn compiles_source_bytes_and_uses_the_raw_size_in_pyc_headers() {
        let source = b"\xef\xbb\xbf# coding: utf-8\r\nanswer = 42\r\n";
        let pyc = compile_bytes_to_pyc(source, "answer.py", 123).unwrap();

        assert_eq!(&pyc[8..12], &123_u32.to_le_bytes());
        assert_eq!(
            &pyc[12..16],
            &u32::try_from(source.len()).unwrap().to_le_bytes()
        );
        assert_eq!(
            &pyc[16..],
            compile("# coding: utf-8\nanswer = 42\n", "answer.py")
                .unwrap()
                .marshal()
        );
    }

    #[test]
    fn exposes_source_decoding_without_filesystem_io() {
        assert_eq!(
            decode_source(b"\xef\xbb\xbf# coding: utf-8\r\nvalue = '\xc3\xa9'\r").unwrap(),
            "# coding: utf-8\nvalue = '\u{e9}'\n"
        );
        assert_eq!(
            decode_source(b"# coding: latin-1\nvalue = '\xe9'\n").unwrap(),
            "# coding: latin-1\nvalue = '\u{e9}'\n"
        );
        assert!(compile("# coding: latin-1\nvalue = 'already decoded'\n", "value.py").is_ok());
    }

    #[test]
    fn distinguishes_a_byte_order_mark_in_text_and_byte_source() {
        use serc::{
            CompileOptions, InitialInternState, PythonVersion, compile_bytes_with_options,
            compile_with_intern_state, compile_with_options,
        };

        let text = "\u{feff}value = 1\r\n";
        let expected_message = "invalid non-printable character U+FEFF";
        let assert_rejected = |error: CompileError| {
            assert!(error.to_string().contains(expected_message), "{error}");
            assert_eq!(error.range().unwrap().start().to_u32(), 0);
            assert_eq!(error.range().unwrap().end().to_u32(), 3);
        };
        assert_rejected(compile(text, "bom.py").unwrap_err());
        assert_rejected(compile_to_pyc(text, "bom.py", 0).unwrap_err());
        assert_rejected(
            compile_with_intern_state(text, "bom.py", &InitialInternState::default()).unwrap_err(),
        );
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
            assert_rejected(compile_with_options(text, "bom.py", &options).unwrap_err());
            assert_eq!(
                compile_bytes_with_options(text.as_bytes(), "bom.py", &options)
                    .unwrap()
                    .marshal(),
                compile_with_options("value = 1\n", "bom.py", &options)
                    .unwrap()
                    .marshal(),
            );
        }

        let output = super::common::python_3145_command()
            .args([
                "-c",
                "try:\n compile(chr(0xfeff) + 'value = 1\\n', 'bom.py', 'exec')\nexcept SyntaxError as error:\n print(error.msg, error.lineno, error.offset)\nelse:\n raise AssertionError('accepted text BOM')",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            format!("{expected_message} 1 1"),
        );
        assert_matches_cpython_marshal_bytes(text.as_bytes(), "bom.py");
        assert_matches_cpython_marshal("# \u{feff}\nvalue = '\u{feff}'\n", "literal_bom.py");
    }

    #[test]
    fn rejects_null_characters_in_text_source() {
        use serc::{
            CompileOptions, InitialInternState, PythonVersion, SourceDecodeError,
            compile_with_intern_state, compile_with_options,
        };

        let expected = CompileError::Decode(SourceDecodeError::NullByte);
        for source in [
            "\0value = 1\n",
            "# generated comment\0\nvalue = 1\n",
            "value = 'before\0after'\n",
            "value = 1\n\0",
            "\u{feff}# generated comment\0\r\nvalue = 1\r\n",
        ] {
            assert_eq!(compile(source, "null.py").unwrap_err(), expected);
            assert_eq!(compile_to_pyc(source, "null.py", 0).unwrap_err(), expected);
            assert_eq!(
                compile_with_intern_state(source, "null.py", &InitialInternState::default())
                    .unwrap_err(),
                expected
            );
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
                assert_eq!(
                    compile_with_options(source, "null.py", &options).unwrap_err(),
                    expected
                );
            }
        }

        assert_matches_cpython_marshal("value = '\\0'\n", "escaped_null.py");
    }

    #[test]
    fn rejects_null_characters_in_filenames_before_reading_source() {
        use serc::{
            CompileOptions, InitialInternState, PythonVersion, SourceDecodeError,
            compile_bytes_with_intern_state, compile_bytes_with_options, compile_with_intern_state,
            compile_with_options,
        };

        let expected = CompileError::InvalidFilename;
        assert_eq!(expected.to_string(), "embedded null character");
        assert_eq!(expected.range(), None);
        assert!(expected.source().is_none());

        let initial_intern_state = InitialInternState::default();
        let text_sources = ["pass", "if =", "\0"];
        let byte_sources: &[&[u8]] =
            &[b"pass", b"if =", b"\0", b"\xff", b"# coding: unknown\npass"];
        for filename in ["\0", "\0prefix", "in\0side", "suffix\0"] {
            for source in text_sources {
                assert_eq!(compile(source, filename).unwrap_err(), expected);
                assert_eq!(compile_to_pyc(source, filename, 0).unwrap_err(), expected);
                assert_eq!(
                    compile_with_intern_state(source, filename, &initial_intern_state).unwrap_err(),
                    expected
                );
            }
            for &source in byte_sources {
                assert_eq!(compile_bytes(source, filename).unwrap_err(), expected);
                assert_eq!(
                    compile_bytes_to_pyc(source, filename, 0).unwrap_err(),
                    expected
                );
                assert_eq!(
                    compile_bytes_with_intern_state(source, filename, &initial_intern_state)
                        .unwrap_err(),
                    expected
                );
            }
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
                for source in text_sources {
                    assert_eq!(
                        compile_with_options(source, filename, &options).unwrap_err(),
                        expected
                    );
                }
                for &source in byte_sources {
                    assert_eq!(
                        compile_bytes_with_options(source, filename, &options).unwrap_err(),
                        expected
                    );
                }
            }
        }

        for filename in [
            "",
            "plain.py",
            "literal\\0.py",
            "near␀nul.py",
            "folder\nname.py",
        ] {
            assert_matches_cpython_marshal("pass", filename);
            assert_matches_cpython_marshal_bytes(b"pass", filename);
        }
        assert!(matches!(
            compile("if =", "normal.py"),
            Err(CompileError::Parse(_))
        ));
        assert_eq!(
            compile_bytes(b"\0", "normal.py").unwrap_err(),
            CompileError::Decode(SourceDecodeError::NullByte)
        );

        let oracle = super::common::python_3145_command()
            .args([
                "-c",
                r##"
for filename in ["\0", "\0prefix", "in\0side", "suffix\0"]:
    for source in ["pass", "if =", "\0", b"pass", b"if =", b"\0", b"\xff", b"# coding: unknown\npass"]:
        try:
            compile(source, filename, "exec")
        except ValueError as error:
            assert str(error) == "embedded null character", str(error)
        else:
            raise AssertionError((filename, source))
"##,
            ])
            .output()
            .unwrap();
        assert!(
            oracle.status.success(),
            "{}",
            String::from_utf8_lossy(&oracle.stderr)
        );
    }

    #[test]
    fn rejects_null_characters_in_path_filenames_before_reading_source() {
        use std::path::Path;

        use serc::{
            CompileOptions, PythonVersion, SourceDecodeError, compile_bytes_with_options,
            compile_bytes_with_path_and_options,
        };

        let sources: &[&[u8]] = &[b"pass", b"if =", b"\0", b"\xff", b"# coding: unknown\npass"];
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
            for filename in ["\0", "\0prefix", "in\0side", "suffix\0"] {
                for &source in sources {
                    assert_eq!(
                        compile_bytes_with_path_and_options(source, Path::new(filename), &options)
                            .unwrap_err(),
                        CompileError::InvalidFilename,
                        "{python_version:?}, {filename:?}, {source:?}"
                    );
                }
            }

            let source = b"def nested():\n    return 42\n";
            for filename in ["", "plain.py", "literal\\0.py", "near␀nul.py"] {
                assert_eq!(
                    compile_bytes_with_path_and_options(source, Path::new(filename), &options)
                        .unwrap()
                        .marshal(),
                    compile_bytes_with_options(source, filename, &options)
                        .unwrap()
                        .marshal(),
                    "{python_version:?}, {filename:?}"
                );
            }

            let filename = Path::new("normal.py");
            assert!(matches!(
                compile_bytes_with_path_and_options(b"if =", filename, &options),
                Err(CompileError::Parse(_))
            ));
            assert_eq!(
                compile_bytes_with_path_and_options(b"\0", filename, &options).unwrap_err(),
                CompileError::Decode(SourceDecodeError::NullByte)
            );
            assert!(matches!(
                compile_bytes_with_path_and_options(b"\xff", filename, &options),
                Err(CompileError::Decode(SourceDecodeError::InvalidUtf8(_)))
            ));
            assert_eq!(
                compile_bytes_with_path_and_options(b"# coding: unknown\npass", filename, &options)
                    .unwrap_err(),
                CompileError::Decode(SourceDecodeError::UnsupportedEncoding("unknown".to_owned()))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn matches_cpython_marshal_for_in_memory_unix_path_filenames() {
        use std::ffi::OsStr;
        use std::io::Write as _;
        use std::os::unix::ffi::OsStrExt as _;
        use std::path::Path;
        use std::process::{Command, Stdio};

        use serc::{CompileOptions, PythonVersion, compile_bytes_with_path_and_options};

        let source = b"def outer():\n    def inner():\n        return 42\n    return inner\n";
        let filenames: &[&[u8]] = &[
            b"plain.py",
            b"nested/path.py",
            b"caf\xc3\xa9.py",
            b"cafe\xcc\x81.py",
            b"replacement-\xef\xbf\xbd.py",
            b"invalid-\xff.py",
            b"continuation-\x80\xbf.py",
            b"truncated-\xe2\x82.py",
            b"surrogate-\xed\xa0\x80.py",
            b"mixed-\xc3\xa9-\xff-\xf0\x9f\x90\xa7.py",
        ];
        let oracle_script = r"
import importlib.util, marshal, os, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[2]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[3]
filename = os.fsdecode(bytes.fromhex(sys.argv[1]))
code = compile(sys.stdin.buffer.read(), filename, 'exec', dont_inherit=True, optimize=0)
def assert_filenames(code):
    assert code.co_filename is filename
    for constant in code.co_consts:
        if isinstance(constant, types.CodeType):
            assert_filenames(constant)
assert_filenames(code)
sys.stdout.buffer.write(marshal.dumps(code))
";

        for (python_version, variable, default) in [
            (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
            (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
            (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
            (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
        ] {
            let executable = std::env::var_os(variable).unwrap_or_else(|| default.into());
            let target = python_version.target();
            let options = CompileOptions {
                python_version,
                ..CompileOptions::default()
            };
            for &filename in filenames {
                let mut hex = String::new();
                for byte in filename {
                    write!(hex, "{byte:02x}").unwrap();
                }
                let mut oracle = Command::new(&executable)
                    .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle_script])
                    .args([
                        &hex,
                        target.tag.strip_prefix('v').unwrap(),
                        &target.magic_hex(),
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap_or_else(|error| panic!("could not start {executable:?}: {error}"));
                oracle.stdin.take().unwrap().write_all(source).unwrap();
                let expected = oracle.wait_with_output().unwrap();
                assert!(
                    expected.status.success(),
                    "{python_version}, {hex}: {}",
                    String::from_utf8_lossy(&expected.stderr),
                );
                let actual = compile_bytes_with_path_and_options(
                    source,
                    Path::new(OsStr::from_bytes(filename)),
                    &options,
                )
                .unwrap()
                .marshal();
                assert_eq!(actual, expected.stdout, "{python_version}, {hex}");
            }
        }
    }

    #[test]
    fn matches_cpython_marshal_for_supported_source_encodings() {
        let cases: [(&str, &[u8]); 9] = [
            ("latin_1.py", b"# coding: latin-1\nvalue = '\xe9'\n"),
            ("euc_jp.py", b"# coding: euc-jp\nvalue = '\xa1\xc1'\n"),
            ("koi8_r.py", b"# coding: koi8-r\nvalue = '\xe1'\n"),
            ("cp1251.py", b"# coding: cp1251\nvalue = '\xc0'\n"),
            ("iso_8859_1.py", b"# coding: iso.8859.1\nvalue = '\xe9'\n"),
            ("iso_ir_100.py", b"# coding: iso.ir.100\nvalue = '\xe9'\n"),
            (
                "iso_8859_15.py",
                b"#!/usr/bin/env python\n# -*- coding: iso-8859-15 -*-\nvalue = '\xa4\xa6\xa8\xb4\xb8\xbc\xbd\xbe'\n",
            ),
            ("u_jis.py", b"# coding: u.jis\nvalue = '\xa1\xc1'\n"),
            (
                "windows_1251.py",
                b"# coding: windows.1251\nvalue = '\xc0'\n",
            ),
        ];
        for (filename, source) in cases {
            assert_matches_cpython_marshal_bytes(source, filename);
        }

        let mut latin9 = b"# coding: latin9\nvalue = '".to_vec();
        latin9.extend(0x80..=0xff);
        latin9.extend_from_slice(b"'\n");
        assert_matches_cpython_marshal_bytes(&latin9, "latin9_all_bytes.py");
    }

    #[test]
    fn normalizes_byte_source_bom_and_text_source_newlines_before_compiling() {
        let source =
            "\u{feff}def greet(name):\r\n    message = f\"héllo, {name}\"\r    return message\r\n";
        let normalized = "def greet(name):\n    message = f\"héllo, {name}\"\n    return message\n";

        assert_eq!(
            compile_bytes(source.as_bytes(), "greet.py")
                .unwrap()
                .marshal(),
            compile(normalized, "greet.py").unwrap().marshal()
        );
        assert_eq!(
            compile(source.trim_start_matches('\u{feff}'), "greet.py")
                .unwrap()
                .marshal(),
            compile(normalized, "greet.py").unwrap().marshal()
        );
    }

    #[test]
    fn structured_parse_error_preserves_ruff_context() {
        let source = "if True print('no')\n";
        let error = compile(source, "invalid.py").unwrap_err();

        let CompileError::Parse(parse_error) = &error else {
            panic!("expected Ruff parse error, got {error:?}");
        };
        assert_eq!(error.range(), Some(parse_error.location));
        assert_eq!(&source[parse_error.location], "print");

        let source_error = error.source().unwrap();
        assert!(source_error.is::<ruff_python_parser::ParseError>());
        assert!(source_error.source().is_some());
        assert_eq!(
            error.to_string(),
            format!("failed to parse Python source: {parse_error}")
        );
    }

    #[test]
    fn rejects_lazy_imports_for_the_pinned_python_target() {
        for source in [
            "lazy import module\n",
            "lazy from module import value\n",
            "if False:\n    if ready:\n        lazy import module\n",
            "def function():\n    lazy import module\n",
            "class Class:\n    lazy from module import value\n",
            "try:\n    lazy import module\nexcept ImportError:\n    pass\n",
        ] {
            for result in [
                compile(source, "lazy_import.py"),
                compile_bytes(source.as_bytes(), "lazy_import.py"),
            ] {
                let error = result.unwrap_err();
                let CompileError::InvalidSyntax { message, range } = &error else {
                    panic!("expected invalid syntax error for {source:?}, got {error:?}");
                };
                assert_eq!(
                    message,
                    "Cannot use `lazy` import statement on Python 3.14 (syntax was added in Python 3.15)"
                );
                assert_eq!(error.range(), Some(*range));
                assert_eq!(&source[*range], "lazy");
            }
        }
    }

    #[test]
    fn lazy_remains_valid_as_an_identifier_and_module_name() {
        let source = "lazy = 1\nimport lazy\nfrom lazy import value\nfrom module import lazy\n";
        assert_matches_cpython_marshal(source, "lazy_identifier.py");
    }

    #[test]
    fn structured_parse_error_separates_invalid_patterns() {
        let source = "match value:\n    case [left] | [right]:\n        pass\n";
        let error = compile(source, "invalid_pattern.py").unwrap_err();

        let CompileError::InvalidPattern { message, range } = &error else {
            panic!("expected invalid pattern error, got {error:?}");
        };
        assert_eq!(message, "alternative patterns bind different names");
        assert_eq!(error.range(), Some(*range));
        assert_eq!(&source[*range], "[left] | [right]");
        assert!(error.source().is_none());
        assert_eq!(
            error.to_string(),
            "failed to parse Python source: alternative patterns bind different names"
        );
    }

    #[test]
    fn structured_parse_error_ranges_duplicate_pattern_bindings() {
        let source = "match value:\n    case [x, [x] | [x]]:\n        pass\n";
        let error = compile(source, "duplicate_pattern.py").unwrap_err();

        let CompileError::InvalidPattern { message, range } = &error else {
            panic!("expected invalid pattern error, got {error:?}");
        };
        assert_eq!(message, "multiple assignments to name 'x' in pattern");
        assert_eq!(&source[*range], "[x] | [x]");
    }

    #[test]
    fn structured_semantic_syntax_errors_have_ranges() {
        let cases = [
            ("break\n", "'break' outside loop", "break"),
            ("continue\n", "'continue' not properly in loop", "continue"),
            ("return value\n", "'return' outside function", "return"),
            ("await value\n", "'await' outside function", "await"),
            (
                "async for item in items:\n    pass\n",
                "'async for' outside async function",
                "async for",
            ),
            ("yield value\n", "'yield' outside function", "yield"),
            (
                "yield from values\n",
                "'yield from' outside function",
                "yield from",
            ),
            (
                "async def f():\n    yield from values\n",
                "'yield from' inside async function",
                "yield from",
            ),
            (
                "while condition:\n    pass\nelse:\n    break\n",
                "'break' outside loop",
                "break",
            ),
            (
                "while condition:\n    def nested():\n        continue\n",
                "'continue' not properly in loop",
                "continue",
            ),
            (
                "def outer():\n    class Nested:\n        return value\n",
                "'return' outside function",
                "return",
            ),
        ];

        for (source, expected_message, expected_prefix) in cases {
            let error = compile(source, "invalid_semantic_syntax.py").unwrap_err();
            let CompileError::InvalidSyntax { message, range } = &error else {
                panic!("expected invalid syntax error, got {error:?}");
            };

            assert_eq!(message, expected_message);
            assert_eq!(error.range(), Some(*range));
            assert!(
                source[*range].starts_with(expected_prefix),
                "expected {range:?} to start with {expected_prefix:?}, got {:?}",
                &source[*range]
            );
            assert!(error.source().is_none());
            assert_eq!(
                error.to_string(),
                format!("failed to parse Python source: {expected_message}")
            );
        }
    }

    #[test]
    fn async_with_is_rejected_consistently_outside_async_functions() {
        let cases = [
            "async with manager:\n    pass\n",
            "async with manager:\n    pass\nafter = 1\n",
            "def f():\n    async with manager:\n        pass\n",
            "def f():\n    async with manager:\n        pass\n    after = 1\n",
        ];

        for source in cases {
            let error = compile(source, "invalid_async_with.py").unwrap_err();
            let CompileError::InvalidSyntax { message, range } = &error else {
                panic!("expected invalid syntax error, got {error:?}");
            };

            assert_eq!(message, "'async with' outside async function");
            assert_eq!(error.range(), Some(*range));
            assert!(source[*range].starts_with("async with"));
        }
    }

    #[test]
    fn valid_semantic_syntax_still_compiles() {
        let source = r"
while condition:
    break

def regular():
    return value

def generator():
    yield value
    yield from values

async def coroutine():
    await value
    async for item in items:
        continue
    async with manager:
        return item
";

        compile(source, "valid_semantic_syntax.py").unwrap();
    }

    #[test]
    fn matches_cpython_marshal_for_iterator_return_collector_lifetimes() {
        let source = include_str!("fixtures/iterator_return_collector_lifetimes.py");
        assert_matches_cpython_marshal(source, "iterator_return_collector_lifetimes.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_empty_module() {
        assert_matches_cpython_marshal("", "empty.py");
    }

    #[test]
    fn benchmark_workloads_match_cpython_marshal() {
        for workload in fixtures::COMPILE_MICRO_WORKLOADS
            .iter()
            .chain(fixtures::COMPOSITE_WORKLOADS)
            .chain(fixtures::LARGE_WORKLOADS)
            .chain(fixtures::MARSHAL_MICRO_WORKLOADS)
        {
            assert_matches_cpython_marshal(workload.source, workload.filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_if_branch_ending_in_while() {
        let source = "def f(condition, index, limit):\n    if condition:\n        value = 1\n        while index < limit:\n            index += 1\n    else:\n        fallback = 1\n        while index > 0:\n            index -= 1\n    tail0 = 0\n    tail1 = 1\n    tail2 = 2\n    tail3 = 3\n    tail4 = 4\n";
        assert_matches_cpython_marshal(source, "if_while.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_module_containing_only_global_statements() {
        let source = "global x\nglobal x, y, z\n";
        assert_matches_cpython_marshal(source, "global.py");
    }

    #[test]
    fn matches_cpython_marshal_when_a_match_case_ends_in_a_constant_expression() {
        let source = fixtures::MATCH_CONTROL_FLOW;
        assert_matches_cpython_marshal(source, "match.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_match_case_ending_in_if() {
        let source = r#"match shopper:
    case "Jane":
        consume(shopper)
        if condition:
            consume(condition)
    case _:
        fallback(shopper)
"#;
        assert_matches_cpython_marshal(source, "match_tail_if.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_match_case_starting_with_if() {
        let source = r"match subject:
    case 1:
        if first or second:
            consume(subject)
    case 2:
        consume(subject)
";
        assert_matches_cpython_marshal(source, "match_leading_if.py");
    }

    #[test]
    fn matches_cpython_marshal_for_global_capture_match_exit_borrowing() {
        let source = r"def capture(subject):
    global captured
    match subject:
        case captured:
            pass
    consume(subject)


def capture_as(subject):
    global captured_as
    match subject:
        case _ as captured_as:
            pass
    consume(subject)
";
        assert_matches_cpython_marshal(source, "global_capture_match_exit.py");
    }

    #[test]
    fn keeps_refutable_and_local_match_exit_borrow_controls() {
        let source = r"def refutable_global_capture(subject):
    global captured
    match (None, subject):
        case (_, captured):
            pass
    consume(subject)


def nested_local_capture(subject):
    match subject:
        case outer:
            match subject:
                case inner:
                    pass
    consume(subject, outer, inner)
";
        assert_matches_cpython_marshal(source, "match_exit_borrow_controls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unpacked_match_capture_swaps() {
        let source = r"def nested_class(value):
    match value:
        case Attribute(value=Name(identifier), attribute=attribute):
            return identifier, attribute
        case _:
            return None


def paired_classes(value):
    match value:
        case (Left() as left, Right() as right):
            return left, right
        case _:
            return None
";
        assert_matches_cpython_marshal(source, "unpacked_match_capture_swaps.py");
    }

    #[test]
    fn matches_cpython_marshal_for_reordered_or_pattern_captures() {
        for pattern in [
            "[left, right] | [right, left]",
            "[left, right] | [\n            right,\n            left\n        ]",
            "[left, middle, right] | [right, left, middle] | [middle, right, left]",
            "{'left': left, 'right': right} | {'left': right, 'right': left}",
            "Point(left, right) | Point(right, left)",
            "[left, right] | (left, right)",
        ] {
            let source = format!(
                "def capture(subject):\n    match subject:\n        case {pattern}:\n            return left, right\n"
            );
            assert_matches_cpython_marshal(&source, "or_pattern_captures.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_or_pattern_failure_cleanup_boundaries() {
        for (pattern, guard) in [
            ("([capture] | [_, capture]) as whole", ""),
            ("([capture] | [_, capture]) as whole", " if subject"),
            ("(([capture] | [_, capture]) as middle) as whole", ""),
            ("(0 | 1) as whole", ""),
        ] {
            let source = format!(
                "def capture(subject):\n    match subject:\n        case {pattern}{guard}:\n            pass\n        case _:\n            return subject\n"
            );
            assert_matches_cpython_marshal(&source, "or_pattern_failure_cleanup.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_match_success_next_locations() {
        let source = r"def tuple_result(subject):
    match subject:
        case 1:
            return make_first(), make_second()
        case 2:
            raise Failure()
        case _:
            return None

async def await_result(subject):
    match subject:
        case 1:
            value = await produce()
            return value
        case 2:
            await consume()
        case _:
            return None

def nested_match(subject, nested):
    match subject:
        case 1:
            match nested:
                case 2:
                    return [nested]
                case _:
                    return []
        case _:
            return None
";
        assert_matches_cpython_marshal(source, "match_success_next_location.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_footer_control_flow() {
        let source = "class WithFooter:\n    with manager:\n        def method(self): ...\n\nclass WildcardFooter:\n    match value:\n        case int(): ...\n        case _:\n            def method(self): ...\n\nclass FoldedBranchFooter:\n    if ...:\n        ...\n    else:\n        with manager:\n            for value in values:\n                def method(self): ...\n";
        assert_matches_cpython_marshal(source, "class_footer.py");
    }

    #[test]
    fn matches_cpython_marshal_for_long_or_pattern_case_boundaries() {
        let source = "match x:\n    case \"abcd\" | \"abcd\" | \"abcd\" :\n        pass\n    case \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\" | \"abcd\":\n        pass\n    case xxxxxxxxxxxxxxxxxxxxxxx:\n        pass\n";
        assert_matches_cpython_marshal(source, "pattern_matching_long.py");
    }

    #[test]
    fn matches_cpython_marshal_for_specialized_none_patterns() {
        let source = r"class Wrapper:
    __match_args__ = ('value',)

def singleton(value):
    match value:
        case None:
            return 0
        case True:
            return 1
        case False:
            return 2
        case 0:
            return 3

def nested(value):
    match value:
        case (None, item):
            return item
        case Wrapper(None):
            return 1
        case None | False:
            return 2

def generator(values):
    for value in values:
        match value:
            case None:
                continue
            case _:
                yield value

async def coroutine(value):
    match value:
        case None:
            return 0
        case True:
            return 1
";
        assert_matches_cpython_marshal(source, "specialized_none_patterns.py");
    }

    #[test]
    fn reports_the_original_duplicate_pattern_binding_name() {
        let source = "class Container:\n    match value:\n        case [__captured, [__captured] | [__captured]]:\n            pass\n";
        let error = compile(source, "duplicate_pattern.py").unwrap_err();

        assert_eq!(
            error.to_string(),
            "failed to parse Python source: multiple assignments to name '__captured' in pattern"
        );
    }

    #[test]
    fn matches_cpython_marshal_for_private_pattern_capture_names() {
        let source = "class Container:\n    def capture(self, value):\n        match value:\n            case [__captured]:\n                return __captured\n        return None\n";
        assert_matches_cpython_marshal(source, "private_pattern.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_unreachable_generic_type_alias() {
        let source = "raise Exception\nassert False\ntype X[T] = T\n";
        assert_matches_cpython_marshal(source, "type_alias.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generic_function_defaults() {
        let source = "def positional[T](value: T = T):\n    pass\n\ndef keyword[T](*, value: T = T):\n    pass\n\ndef both[T](value: T = T, *, other: T = T):\n    pass\n";
        assert_matches_cpython_marshal(source, "generic_defaults.py");
    }

    #[test]
    fn matches_cpython_marshal_for_constant_default_tuple_folding_boundary() {
        fn defaults(count: usize, dynamic_last: bool) -> String {
            (0..count)
                .map(|index| {
                    let value = if dynamic_last && index + 1 == count {
                        "dynamic"
                    } else {
                        "None"
                    };
                    format!("arg{index}={value}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        }

        let mut source = String::new();
        for (name, count, dynamic_last) in [
            ("folded_function", 30, false),
            ("unfolded_function", 31, false),
            ("mixed_function", 30, true),
        ] {
            writeln!(
                source,
                "def {name}({}):\n    return None\n",
                defaults(count, dynamic_last)
            )
            .unwrap();
        }
        for (name, count, dynamic_last) in [
            ("folded_lambda", 30, false),
            ("unfolded_lambda", 31, false),
            ("mixed_lambda", 31, true),
        ] {
            writeln!(
                source,
                "{name} = lambda {}: None",
                defaults(count, dynamic_last)
            )
            .unwrap();
        }

        assert_matches_cpython_marshal(&source, "large_constant_defaults.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_generic_class_scopes() {
        let source = "class Outer[T]:\n    class Inner:\n        function = lambda: None\n        values = (value for value in ())\n\n        def method(self, value: T) -> T:\n            return value\n\n        def generic[U](self, value: U) -> U:\n            return value\n";
        assert_matches_cpython_marshal(source, "nested_generic.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generic_class_private_name_contexts() {
        let source = r"class Outer:
    def define():
        class Child[__T]:
            value = __T

        return Child


class ModuleLevel[__V]:
    value = __V
";
        assert_matches_cpython_marshal(source, "generic_class_private_names.py");
    }

    #[test]
    fn preserves_enclosing_private_name_context_for_generic_functions() {
        let source = r"class Outer:
    def define():
        def function[__U]():
            return __U

        return function
";
        assert_matches_cpython_marshal(source, "generic_function_private_names.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generic_wrapper_closures() {
        let source = r"def outer():
    value = int

    class Nested[T]:
        item: value

    return Nested


class Generic[T]:
    def method[U](self):
        return T, U


class ____PrivateContainer:
    def outer(self):
        __value = int

        def generic[T: __value]() -> __value:
            return T, __value

        return generic
";
        assert_matches_cpython_marshal(source, "generic_closures.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_function_decorator_closures() {
        let source = r"def method_decorators(decorator, decorator_factory):
    class Container:
        @decorator
        def method(self):
            pass

        @classmethod
        @decorator
        def class_method(cls):
            pass

        @decorator
        @staticmethod
        def static_method():
            pass

        @decorator_factory(1)
        async def coroutine(self):
            pass

    return Container


def nested_function_decorator(decorator):
    def parent():
        @decorator
        def child():
            pass

        return child

    return parent


def shadowed_class_decorator(decorator, local):
    class Container:
        decorator = local

        @decorator
        def method(self):
            pass

    return Container
";
        assert_matches_cpython_marshal(source, "function_decorator_closures.py");
    }

    #[test]
    fn matches_cpython_marshal_for_decorator_bindings_and_delete_only_locals() {
        let source = r"def bindings():
    @decorator
    def function():
        pass

    @(assigned := decorator)
    class Class:
        pass

    del deleted
    decorator = identity
";
        assert_matches_cpython_marshal(source, "decorator_bindings.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_definition_headers() {
        let source = r"def ordering():
    @decorator
    def function(positional=(function_target := function_source), *, keyword=keyword_source):
        pass

    @class_decorator
    class Class((base_target := base_source), metaclass=metaclass_source):
        pass

    function_source = keyword_source = None
    base_source = object
    metaclass_source = type
    decorator = class_decorator = identity


def function_default_closure(value):
    def parent():
        def plain(item=value):
            return item

        def generic[T](item=value):
            return item

        return plain, generic

    return parent


def class_decorator_closure(decorator):
    def parent():
        @decorator
        class Child:
            pass

        return Child

    return parent
";
        assert_matches_cpython_marshal(source, "definition_headers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_plain_class_header_closures() {
        let source = r"def function_headers(base, metaclass, bases, options, captured):
    def parent():
        class Direct(base, metaclass=metaclass):
            pass

        class Expanded(*bases, **options):
            pass

        class Nested((lambda: captured)()):
            pass

        return Direct, Expanded, Nested

    return parent


def class_headers(base, metaclass, bases, options):
    class Container:
        class Direct(base, metaclass=metaclass):
            pass

        class Expanded(*bases, **options):
            pass

    return Container


def local_headers(base, metaclass):
    def parent(base, metaclass):
        class Child(base, metaclass=metaclass):
            pass

        return Child

    return parent


def shadowed_class_headers(base, metaclass, local_base, local_meta):
    class Container:
        base = local_base
        metaclass = local_meta

        class Child(base, metaclass=metaclass):
            pass

    return Container


def generic_headers(base, metaclass, bound, default):
    def parent():
        class Child[T: bound = default](base, metaclass=metaclass):
            value: T

        return Child

    return parent


def generic_type_parameter_shadow(T):
    def parent():
        class Child[T](T):
            pass

        return Child

    return parent


class Global(global_base, metaclass=global_meta):
    pass
";
        assert_matches_cpython_marshal(source, "class_header_closures.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_body_lambda_closures() {
        let source = r"def self_reference():
    class Local:
        factory = lambda: Local

    return Local


def nested_self_reference():
    class Local:
        factory = lambda: (lambda: Local)

    return Local


def enclosing_reference():
    captured = object()

    class Local:
        factory = lambda: captured

    return Local


def class_local_shadow():
    class Local:
        Local = object()
        factory = lambda: Local

    return Local


def method_control():
    class Local:
        def factory(self):
            return Local

    return Local


def direct_body_control():
    class Local:
        value = Local

    return Local


def function_global_control():
    global GlobalLocal

    class GlobalLocal:
        factory = lambda: GlobalLocal

    return GlobalLocal


def class_global_control():
    captured = object()

    class Local:
        global captured
        factory = lambda: captured

    return Local


def class_nonlocal_control():
    captured = object()

    class Local:
        nonlocal captured
        factory = lambda: captured

    return Local


def dunder_class_control():
    class Local:
        factory = lambda: __class__

    return Local


class ModuleControl:
    factory = lambda: ModuleControl


class ClassParentControl:
    class Local:
        factory = lambda: Local
";
        assert_matches_cpython_marshal(source, "class_body_lambda_closures.py");
    }

    #[test]
    fn matches_cpython_marshal_for_lambda_default_closures() {
        let source = r"def class_self_default():
    class Local:
        thunk = lambda value=Local: value

    return Local


def class_enclosing_default():
    captured = object()

    class Local:
        thunk = lambda value=captured: value

    return Local


def class_nested_lambda_default():
    class Local:
        thunk = lambda value=(lambda: Local): value

    return Local


def function_enclosing_default(captured):
    def parent():
        thunk = lambda value=captured: value
        return thunk

    return parent


def nested_lambda_body_default(captured):
    return lambda: (lambda value=captured: value)


def nested_lambda_nested_default(captured):
    return lambda: (lambda value=(lambda: captured): value)


def nested_lambda_generator_default(captured):
    return lambda: (lambda value=(captured for _ in ()): value)


def nested_lambda_local_default():
    return lambda captured: (lambda value=captured: value)


def class_local_before_control():
    captured = object()

    class Local:
        captured = object()
        thunk = lambda value=captured: value

    return Local


def class_local_after_control():
    captured = object()

    class Local:
        thunk = lambda value=captured: value
        captured = object()

    return Local


def class_body_control():
    class Local:
        thunk = lambda: Local

    return Local


def function_global_control():
    global GlobalLocal

    class GlobalLocal:
        thunk = lambda value=GlobalLocal: value

    return GlobalLocal


class ModuleControl:
    thunk = lambda value=ModuleControl: value


class ClassParentControl:
    class Local:
        thunk = lambda value=Local: value


def default_parameter_name_control():
    class Local:
        thunk = lambda first, second=first: second

    return Local
";
        assert_matches_cpython_marshal(source, "lambda_default_closures.py");
    }

    #[test]
    fn matches_cpython_marshal_for_closure_load_fusion_boundaries() {
        let source = "def tuple_value(label, url):\n    return label, lambda: url\n\ndef keyword_call(label, url):\n    return register(label=label, command=lambda: open(url))\n";
        assert_matches_cpython_marshal(source, "closure_fusion.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_nested_comprehension_captures() {
        let source = include_str!("fixtures/generator_nested_comprehension_captures.py");
        assert_matches_cpython_marshal(source, "generator_nested_comprehension_captures.py");
        assert_executes(&format!("{source}\nprint(list(empty()))\n"), "[(42, [])]\n");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_targets_shadowing_enclosing_freevars() {
        let source = r"def lambda_parent(all_elements, element):
    return lambda owner: all(owner.contains(element) for element in all_elements)

def function_parent(all_elements, element):
    def child(owner):
        return all(owner.contains(element) for element in all_elements)
    return child

def lambda_in_generator_parent(items, predicate):
    def child():
        return any((lambda: predicate(item))() for item in items)
    return child

def comprehension_shadow_parent(items, captured):
    def child():
        values = [captured for captured in items]
        generated = (item for item in items)
        return captured, values, generated
    return child
";
        assert_matches_cpython_marshal(source, "generator_target_shadow.py");
    }

    #[test]
    fn matches_cpython_marshal_for_decorated_generic_function_closures() {
        let source = r"class Container:
    @decorator
    def method[T](self, value: T) -> T: ...

def parent():
    captured = object()
    @decorator
    async def function[T: captured](value: T) -> T: ...
    return function
";
        assert_matches_cpython_marshal(source, "decorated_generic_closure.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_annotation_comprehensions() {
        for source in [
            "class C:\n    def f(x: [i for i in (1,)]): pass\n",
            "class C:\n global __class__\n def f(a: [i for i in __class__]): pass\n",
            "class C:\n global __class__\n a: [i for i in __class__]\n",
            "class C:\n global __class__\n type A = [i for i in __class__]\n",
            "def outer(x):\n class C:\n  global x\n  def f(a: [x for i in (1,)]): pass\n return C\n",
            "def outer(x):\n class C:\n  global x\n  a: [x for i in (1,)]\n return C\n",
            "def outer(x):\n class C:\n  global x\n  def f(a: (x, [x for i in (1,)])): pass\n return C\n",
            "def outer(x):\n class C:\n  global x\n  a: (x, [x for i in (1,)])\n return C\n",
            "class C:\n global __class__\n def f(a: [__class__ for i in (1,)]): pass\n",
            "class C:\n global __class__\n a: [__class__ for i in (1,)]\n",
            "class C:\n global T\n def f[T](a: [T for i in (1,)]): pass\n",
            "class C:\n global T\n def f[T](a: (T, [T for i in (1,)])): pass\n",
            "class C:\n    x: {i for i in (1, 2) if i}\n",
            "class C:\n    def f(x: {i: i + 1 for i in (1, 2) if i}): pass\n",
            "class C:\n    type A = [(i, j) for i in (1, 2) for j in (i,) if j]\n",
            "def outer(value):\n    class C:\n        type A = [value + i for i in (1, 2)]\n    return C\n",
            "class C:\n    type A = [lambda: i for i in (1, 2)]\n",
            "class C:\n    type A = [[lambda: j for j in (i,)] for i in (1, 2)]\n",
            "class C:\n    values = (1, 2)\n    item = int\n    type A = [item for i in values]\n",
            "class C:\n    def f[T: [i for i in (1, 2)]](): pass\n",
            "class C:\n    type A[T = {i for i in (1, 2)}] = T\n",
            "class C:\n    type A = [i for i in [j for j in (1, 2)]]\n",
            "def outer(value):\n    class C:\n        value = 10\n        type A = [value + i for i in (1,)]\n    return C\n",
            "def outer(value):\n    class C:\n        value = 10\n        def f(x: [value + i for i in (1,)]): pass\n    return C\n",
            "def outer(value):\n    class C:\n        value = 10\n        x: [value + i for i in (1,)]\n    return C\n",
            "class C:\n    type A = [__class__ for i in (1,)]\n",
            "class C:\n    def f(x: [__class__ for i in (1,)]): pass\n",
            "class C:\n    x: [__class__ for i in (1,)]\n",
            "def outer(__class__):\n    class C:\n        type A = [i for i in __class__]\n    return C\n",
            "def outer(T):\n    class C:\n        def f[T](x: [T for i in (1,)]): pass\n    return C\n",
            "def outer(a):\n    class C:\n        a = type\n        class D[T](*[a for i in (1,)]): pass\n    return C\n",
        ] {
            assert_matches_cpython_marshal(source, "class_annotation_comprehensions.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_annotation_scope_closures() {
        let source = fixtures::ANNOTATION_SCOPES;
        assert_matches_cpython_marshal(source, "annotation_scopes.py");
    }

    #[test]
    fn matches_cpython_marshal_for_annotation_only_local_captured_by_function_annotation() {
        let source = r"def outer():
    value = 7
    def middle():
        value: int
        def inner(item: value) -> value:
            pass
        return inner
    return middle()

try:
    print(outer().__annotations__)
except NameError:
    print('unbound')
";
        assert_matches_cpython_marshal(source, "annotation_only_local_capture.py");
        assert_executes(source, "unbound\n");

        let future_source = format!("from __future__ import annotations\n{source}");
        assert_matches_cpython_marshal(&future_source, "future_annotation_only_local.py");
        assert_executes(&future_source, "{'item': 'value', 'return': 'value'}\n");

        let private_source = r"class C:
    def outer(self):
        __value = 7
        def middle():
            __value: int
            def inner(item: __value):
                pass
            return inner
        return middle()

try:
    print(C().outer().__annotations__)
except NameError:
    print('unbound')
";
        assert_matches_cpython_marshal(private_source, "private_annotation_only_local.py");
        assert_executes(private_source, "unbound\n");
    }

    #[test]
    fn matches_cpython_marshal_for_class_type_aliases() {
        let source = "from __future__ import annotations\n\nclass Container:\n    Local = int\n    type Alias = Local\n    type Generic[T] = tuple[Local, T]\n    type Factory[T] = lambda: (Local, T)\n";
        assert_matches_cpython_marshal(source, "class_alias.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_global_qualnames() {
        let source = include_str!("fixtures/class_global_qualnames.py");
        assert_matches_cpython_marshal(source, "class_global_qualnames.py");
    }

    #[test]
    fn matches_cpython_marshal_for_global_annotation_scope_qualnames() {
        let source = r"def outer():
    global PlainAlias, GenericAlias, GenericFunction, GenericClass

    type PlainAlias = int
    type GenericAlias[T: int] = tuple[T, T]

    def GenericFunction[T: int]():
        pass

    class GenericClass[T: int]:
        pass
";
        assert_matches_cpython_marshal(source, "global_annotation_qualnames.py");
    }

    #[test]
    fn matches_cpython_marshal_for_string_slice_constants() {
        let source = "subject[\"fine\":\"fine\"]\nsubject[\"x\":\"x\"]\n";
        assert_matches_cpython_marshal(source, "slice.py");
    }

    #[test]
    fn matches_cpython_marshal_for_empty_string_slice_bounds() {
        for source in [
            "subject['':]\n",
            "subject[:'']\n",
            "subject[::'']\n",
            "subject['':'':'']\n",
            "subject['':'%']\n",
            "subject['':'fine':'é']\nsubject['x':'': 'Ā']\n",
            "before = ('', '-')\nsubject['':'-']\n",
            "subject['':'-']\nafter = ('', '-')\n",
            "before = ''\nsubject['':]\nsubject[:'']\nsubject[::'']\nafter = ''\n",
            "def first(subject):\n    return subject['':]\ndef second(subject):\n    return '', subject[:'']\n",
        ] {
            assert_matches_cpython_marshal(source, "empty_string_slice.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_origin_sensitive_frozenset_identity() {
        let source = r#"def membership_first(kind):
    if kind in {None, "sort"}:
        return 1
    return 0


def membership_second(kind):
    if kind in {None, "sort"}:
        return 2
    return 0


def nonstatic_membership_first(kind):
    if kind in {None, "linux"}:
        return 3
    return 0


def nonstatic_membership_second(kind):
    if kind in {None, "linux"}:
        return 4
    return 0


def shared_membership_first(kind):
    return kind in {None, "sort"}


def shared_membership_second(kind):
    return kind in {None, "sort"}


def direct_first():
    value = {"left", "center", "right"}
    return value, 1


def direct_second():
    value = {"left", "center", "right"}
    return value, 2


def direct_static_first():
    value = {"set", "no", "longer", "accepted"}
    return value, 1


def direct_static_second():
    value = {"set", "no", "longer", "accepted"}
    return value, 2


def identifier_seed():
    cubic = 1
    return cubic


def identifier_direct_first():
    value = {"cubic", "quintic"}
    return value, 1


def identifier_direct_second():
    value = {"cubic", "quintic"}
    return value, 2


def cached_character_first():
    value = {"a", "b", "c"}
    return value, 1


def cached_character_second():
    value = {"a", "b", "c"}
    return value, 2


def empty_string_first():
    value = {"", ".", "/"}
    return value, 1


def empty_string_second():
    value = {"", ".", "/"}
    return value, 2
"#;
        assert_matches_cpython_marshal(source, "origin_sensitive_frozensets.py");
    }

    #[test]
    fn matches_cpython_marshal_for_macro_parameter_as_constant() {
        let source = r"def first(value):
    return value in {'NAME', 'novel_marker'}, 1

def second(value):
    'second docstring'
    return value in {'NAME', 'novel_marker'}, 2
";
        assert_matches_cpython_marshal(source, "macro_parameter.py");
    }

    #[test]
    fn matches_cpython_marshal_for_startup_interned_frozenset_members() {
        let source = r#"def startup_first(value):
    if value in {None, "500", "ascii", "darwin", "kwargs", "l1", "l2", "update", "win32"}:
        return 1
    return 0


def startup_second(value):
    if value in {None, "500", "ascii", "darwin", "kwargs", "l1", "l2", "update", "win32"}:
        return 2
    return 0


def ordinary_first(value):
    if value in {None, "alpha", "elasticnet"}:
        return 3
    return 0


def ordinary_second(value):
    if value in {None, "alpha", "elasticnet"}:
        return 4
    return 0
"#;
        assert_matches_cpython_marshal(source, "startup_interned_frozensets.py");
    }

    #[test]
    fn matches_cpython_marshal_for_noninterned_constant_metadata_collisions() {
        let source = r#"from package.config import exported, café
from other import *

if error.name == "package.config":
    recover()

class Outer:
    class Inner:
        pass
"#;
        assert_matches_cpython_marshal(source, "string_metadata_collision.py");
    }

    #[test]
    fn matches_cpython_marshal_for_qualified_name_constant_provenance() {
        let source = r#"module_name = "<module>"

def outer():
    def nested():
        pass
    return "outer.<locals>.nested"
"#;
        assert_matches_cpython_marshal(source, "qualified_name_constants.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_qualified_name_collisions() {
        let source = r#"ordinary_before = "Before.Nested"

class Before:
    class Nested:
        pass

class After:
    class Nested:
        pass

ordinary_after = "After.Nested"
"#;
        assert_matches_cpython_marshal(source, "class_qualified_name_collision.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_literal_return_locations() {
        let source = r#"def triple_quoted():
    return """alpha
beta
gamma"""


def triple_bytes():
    return b"""alpha
beta"""


def implicit_concatenation(flag):
    if flag:
        return ("left "
                "right")
    return (b"fallback "
            b"bytes")


def parenthesized_string():
    return (
        "parenthesized"
    )


def parenthesized_scalar():
    return (
        42
    )


def one_line_literal():
    return "one line"


def multiline_nonliteral(value):
    return (
        value
    )
"#;
        assert_matches_cpython_marshal(source, "multiline_literal_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unary_not_loop_exit_locations() {
        let source = r"def terminal_call():
    while not predicate():
        work()

def terminal_nested_not():
    while not not predicate():
        work()

def terminal_and(flag):
    while flag and not predicate():
        work()

def protected(lock):
    with lock:
        while not lock.done():
            work()
    work()
";
        assert_matches_cpython_marshal(source, "while_not_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nonfinal_while_exits_in_with_bodies() {
        let source = r"def final_sync(manager, flag):
    with manager:
        while flag:
            step()
    outside()


def followed_sync(manager, flag):
    with manager:
        while flag:
            step()
        inside()
    outside()


def nested_followed_sync(manager, flag, outer):
    with manager:
        if outer:
            while flag:
                step()
            inside()
    outside()


def loop_else_sync(manager, flag):
    with manager:
        while flag:
            step()
        else:
            fallback()
        inside()
    outside()


async def followed_async(manager, flag):
    async with manager:
        while flag:
            step()
        inside()
    outside()
";
        assert_matches_cpython_marshal(source, "nonfinal_while_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_while_exit_markers_in_with_bodies() {
        let source = r"def single_exit(manager, flag):
    with manager:
        while flag:
            step()
    outside()


def multiple_condition_exits(manager, first, second):
    with manager:
        while first and second:
            step()
    outside()


def chained_comparison(manager, low, value, high):
    with manager:
        while low < value < high:
            step()
    outside()


def conditional_expression(manager, condition, first, second):
    with manager:
        while first if condition else second:
            step()
    outside()
";
        assert_matches_cpython_marshal(source, "final_while_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_sync_with_while_breaks() {
        let source = r"def nested_tail_break(manager, enabled, stream):
    with manager:
        if enabled:
            line = '\n'
            position = stream.tell()
            while line:
                line = stream.readline()
                if '[' in line:
                    stream.seek(position)
                    break
                position = stream.tell()
        finish(stream)

def nested_break_only(manager, active, items, stop):
    with manager:
        while active:
            for item in items:
                if stop(item):
                    break
            step()
";
        assert_matches_cpython_marshal(source, "terminal_sync_with_while_break.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_while_exit_markers_in_async_with_bodies() {
        let source = r"async def direct_name(manager, flag):
    async with manager:
        while flag:
            step()
    outside()

async def direct_attribute(manager, resource):
    async with manager:
        while resource.enabled:
            step()
    outside()

async def direct_call(manager, resource):
    async with manager:
        while resource.enabled():
            step()
    outside()

async def multi_edge(manager, first, second):
    async with manager:
        while first and second:
            step()
    outside()

async def sync_then_async_with(sync_manager, async_manager, flag):
    with sync_manager:
        async with async_manager:
            while flag:
                step()
    outside()

async def async_then_sync_with(async_manager, sync_manager, flag):
    async with async_manager:
        with sync_manager:
            while flag:
                step()
    outside()

async def async_with_inner_finally(manager, flag):
    async with manager:
        try:
            while flag:
                step()
        finally:
            cleanup()
    outside()

async def async_with_inner_except(manager, flag):
    async with manager:
        try:
            while flag:
                step()
        except Error:
            cleanup()
    outside()
";
        assert_matches_cpython_marshal(source, "async_with_terminal_while.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_while_exit_markers_in_try_bodies() {
        let source = r"def direct(flag):
    try:
        while flag:
            step()
    except Error:
        recover()
    outside()


def direct_not(ready):
    try:
        while not ready:
            step()
    except Error:
        recover()
    outside()


async def direct_async(flag):
    try:
        while flag:
            await step()
    except Error:
        recover()
    outside()


def followed_in_try(flag):
    try:
        while flag:
            step()
        inside()
    except Error:
        recover()
    outside()


def multiple_exits(first, second):
    try:
        while first and second:
            step()
    except Error:
        recover()
    outside()


def loop_else(flag):
    try:
        while flag:
            step()
        else:
            fallback()
    except Error:
        recover()
    outside()


def try_else(flag):
    try:
        while flag:
            step()
    except Error:
        recover()
    else:
        success()
    outside()


def raising_body(flag):
    try:
        while flag:
            raise Error
    except Error:
        recover()
    outside()


def followed_raising_body(flag):
    try:
        while flag:
            raise Error
        inside()
    except Error:
        recover()
    outside()


def finally_cleanup(flag):
    try:
        while flag:
            step()
    finally:
        cleanup()
    outside()


def terminal(flag):
    try:
        while flag:
            step()
    except Error:
        recover()
";
        assert_matches_cpython_marshal(source, "final_while_try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unary_not_synthetic_exit_locations() {
        let source = r"def branch_join(flag):
    if flag:
        if not predicate():
            return False
    else:
        recover()
    return True


def terminal_assert(flag):
    if flag:
        assert predicate()
    else:
        assert not predicate()


def handler_exit():
    try:
        action()
    except Error as error:
        if not predicate():
            recover()


def finally_exit():
    try:
        action()
    finally:
        if not predicate():
            recover()


def loop_tail():
    while outer():
        if not predicate():
            work()
";
        assert_matches_cpython_marshal(source, "unary_not_synthetic_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_try_at_the_end_of_a_loop() {
        let source = "async def show_status():\n    while True:\n        try:\n            if report_host:\n                data = value\n        except Exception as error:\n            pass\n";
        assert_matches_cpython_marshal(source, "loop_try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_condition_fallthrough_ranges() {
        let source = r"def short_circuit_break():
    while True:
        try:
            if first() or second():
                break
        except BlockingIOError:
            pass


def single_condition_break():
    while True:
        try:
            if first():
                break
        except BlockingIOError:
            pass


def short_circuit_continue():
    while True:
        try:
            if first() or second():
                continue
        except BlockingIOError:
            pass
";
        assert_matches_cpython_marshal(source, "try_condition_fallthrough_ranges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multi_exit_try_loop_tails() {
        let source = r"def conditional(items):
    for item in items:
        try:
            if first(item):
                consume(item)
            elif second(item):
                consume_other(item)
        except LookupError:
            recover(item)


def complete_branch(items):
    for item in items:
        try:
            if accepted(item):
                consume(item)
            else:
                reject(item)
        except LookupError:
            recover(item)


def single_exit_return(items):
    for item in items:
        try:
            if accepted(item):
                return item
        except LookupError:
            continue


def single_exit_break(items):
    for item in items:
        try:
            if accepted(item):
                break
        except LookupError:
            continue
";
        assert_matches_cpython_marshal(source, "try_loop_tail_normal_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_short_circuit_loop_tail_backedges() {
        let source = r"def collect(items):
    output = {}
    for item in items:
        try:
            name = item.name
            version = item.version
            if name and version:
                output[name] = version
        except Exception:
            recover(item)
    return output
";
        assert_matches_cpython_marshal(source, "protected_short_circuit_backedges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_non_final_branch_loop_tails() {
        let source = r"def protected_state_machine(items):
    for index, item in enumerate(items):
        try:
            if state == SCANNING:
                if item.startswith(tag):
                    if item.endswith(end):
                        state = STATE_RETTYPE
                    else:
                        state = STATE_DOC
            elif state == STATE_DOC:
                if item.startswith(end):
                    state = STATE_RETTYPE
                else:
                    output.append(item)
            elif state == STATE_RETTYPE:
                state = STATE_NAME
            elif state == STATE_NAME:
                state = STATE_ARGS
            elif state == STATE_ARGS:
                if item.startswith(start):
                    state = SCANNING
                else:
                    arguments.append(item)
        except ParseError:
            raise
        except Exception as error:
            message = 'details'
            raise ParseError(index, message) from error


def single_normal_exit_control(items):
    for item in items:
        try:
            if accepted(item):
                consume(item)
        except LookupError:
            continue
";
        assert_matches_cpython_marshal(source, "protected_non_final_branch_loop_tails.py");
    }

    #[test]
    fn matches_cpython_marshal_for_shared_artificial_loop_backedges() {
        let source = r"def try_else(values, output):
    for value in values:
        try:
            callback = lookup(value)
        except KeyError:
            pass
        else:
            if callback(value):
                output.append(value)
    return output


def try_finally(values):
    for value in values:
        try:
            consume()
        finally:
            if enabled():
                cleanup()
";
        assert_matches_cpython_marshal(source, "shared_artificial_loop_backedges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_selected_try_else_loop_backedges() {
        for (filename, source) in [
            (
                "backward_join_selected_try_else_conditional_following_call_control.py",
                r#"def collect(rows, parse, output, report, ignore_errors, finish):
    for kind, payload in rows:
        if kind == "record":
            try:
                value = parse(payload)
            except ValueError as error:
                if not ignore_errors:
                    raise error
                report(payload)
            else:
                if value is not None:
                    output.append(value)
            finish()
"#,
            ),
            (
                "backward_join_selected_try_else_conditional_tail.py",
                r#"def collect(rows, parse, output, report, ignore_errors, finish):
    for kind, payload in rows:
        if kind == "record":
            try:
                value = parse(payload)
            except ValueError as error:
                if not ignore_errors:
                    raise error
                report(payload)
            else:
                if value is not None:
                    output.append(value)
"#,
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_try_else_elif_loop_backedges() {
        for (filename, source) in [
            (
                "selected_try_else_elif_chained_backedge.py",
                r#"def collect(rows, output):
    for row in rows:
        if row.enabled:
            try:
                low = float(row.low)
                middle = float(row.middle)
                high = float(row.high)
            except ValueError:
                output.append("invalid input")
            else:
                if not all(is_finite(value) for value in (low, middle, high)):
                    output.append("nonfinite")
                elif not low <= middle <= high:
                    output.append("out of order")
"#,
            ),
            (
                "direct_try_else_elif_chained_backedge.py",
                r#"def collect(rows, output):
    for row in rows:
        try:
            low = float(row.low)
            middle = float(row.middle)
            high = float(row.high)
        except ValueError:
            output.append("invalid input")
        else:
            if not all(is_finite(value) for value in (low, middle, high)):
                output.append("nonfinite")
            elif not low <= middle <= high:
                output.append("out of order")
"#,
            ),
            (
                "selected_try_else_elif_following_call_control.py",
                r#"def collect(rows, output):
    for row in rows:
        if row.enabled:
            try:
                low = float(row.low)
                middle = float(row.middle)
                high = float(row.high)
            except ValueError:
                output.append("invalid input")
            else:
                if not all(is_finite(value) for value in (low, middle, high)):
                    output.append("nonfinite")
                elif not low <= middle <= high:
                    output.append("out of order")
            finish(row)
"#,
            ),
            (
                "selected_try_else_elif_final_else_control.py",
                r#"def collect(rows, output):
    for row in rows:
        if row.enabled:
            try:
                low = float(row.low)
                middle = float(row.middle)
                high = float(row.high)
            except ValueError:
                output.append("invalid input")
            else:
                if not all(is_finite(value) for value in (low, middle, high)):
                    output.append("nonfinite")
                elif not low <= middle <= high:
                    output.append("out of order")
                else:
                    output.append("accepted")
"#,
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_nested_try_else_loop_backedges() {
        let source = r"def nested_try_else_loop(first, second):
    try:
        while True:
            try:
                outer_value = take_outer()
            except OuterError:
                break
            else:
                while True:
                    try:
                        inner_value = take_inner()
                    except InnerError:
                        break
                    else:
                        consume(inner_value)
                        if first:
                            consume(outer_value)
                            if second:
                                consume(second)
    except Exception:
        recover()
    finish()
";
        assert_matches_cpython_marshal(source, "nested_try_else_loop_backedges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_try_else_loop_backedge_controls() {
        let source = r"def final_conditional_else(values):
    for value in values:
        try:
            callback = lookup(value)
        except KeyError:
            pass
        else:
            prepare(callback)
            if callback(value):
                consume(value)
            else:
                reject(value)


def constant_final_condition(values):
    for value in values:
        try:
            callback = lookup(value)
        except KeyError:
            pass
        else:
            prepare(callback)
            if True:
                consume(value)
";
        assert_matches_cpython_marshal(source, "nested_try_else_loop_backedge_controls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_try_suite_implicit_returns() {
        let source = r"def body_if(flag):
    try:
        if flag:
            action()
    except Exception:
        pass

def leading_and_terminal_if(first, second):
    try:
        if first:
            before()
        if second:
            action()
    except Exception:
        pass

def body_if_else(flag):
    try:
        if flag:
            action()
        else:
            other()
    except Exception:
        pass

def body_while(flag):
    try:
        while flag:
            action()
            break
    except Exception:
        pass

def body_match(value):
    try:
        match value:
            case 1:
                action()
    except Exception:
        pass

def else_if(flag):
    try:
        action()
    except Exception:
        recover()
    else:
        if flag:
            success()

def conditional_expression(flag):
    try:
        action() if flag else other()
    except Exception:
        pass

def boolean_expression(flag):
    try:
        flag and action()
    except Exception:
        pass

def chained_comparison(left, middle, right):
    try:
        left < middle < right
    except Exception:
        pass

def tuple_conditional_expression(flag):
    try:
        (before(), action() if flag else other())
    except Exception:
        pass

async def coroutine_if(flag):
    try:
        if flag:
            await action()
    except Exception:
        pass
";
        assert_matches_cpython_marshal(source, "terminal_try_returns.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_assignment_return_boundaries() {
        let source = r"def consecutive_handlers(flag):
    global state
    if flag:
        try:
            first()
        except FirstError:
            pass
        try:
            second()
        except SecondError:
            pass
        state = None


def explicit_else(flag):
    if flag:
        use(flag)
    else:
        try:
            value = first()
        except FirstError:
            pass
        value = None


def call_tail_control(flag):
    if flag:
        try:
            first()
        except FirstError:
            pass
        finish()


def nonterminal_control(flag):
    if flag:
        try:
            first()
        except FirstError:
            pass
        value = None
    finish()


def simple_assignment_control(flag):
    if flag:
        value = None
";
        assert_matches_cpython_marshal(source, "terminal_assignment_return_boundary.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_conditional_assignment_copy_threshold() {
        let source = r#"def direct_name(flag, left, right):
    value = left if flag else right


def direct_attribute(flag, left, right, target):
    target.value = left if flag else right


def subscript(flag, left, right, target):
    target["value"] = left if flag else right


def unpack(flag, left, right):
    first, second = left if flag else right


def chained(flag, left, right):
    first = second = left if flag else right


def complex_attribute(flag, left, right, target):
    target.child.value = left if flag else right


def nested_unpack(flag, left, right):
    [(value,)] = left if flag else right
"#;
        assert_matches_cpython_marshal(source, "terminal_conditional_assignment_threshold.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_terminal_try_condition_ownership() {
        let source = r"def nested_try(a, b):
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

def complex_condition(a, b, c):
    try:
        if a and (b if c else other()):
            action()
    except Exception:
        recover()

def guarded_match(value, flag):
    try:
        match value:
            case 1 if flag and predicate():
                action()
    except Exception:
        recover()

def nested_try_generator(a, b):
    yield 1
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def nested_try_coroutine(a, b):
    await before()
    try:
        if a:
            try:
                b and await action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def nested_try_async_generator(a, b):
    yield 1
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def coroutine_nested_finally(a):
    try:
        if a:
            try:
                action()
            finally:
                cleanup()
    except Exception:
        recover()

async def coroutine_nested_else(a):
    try:
        if a:
            try:
                action()
            except ValueError:
                inner()
            else:
                success()
    except Exception:
        recover()

async def await_late_in_bool_condition(a):
    try:
        if a and await predicate():
            try:
                action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def await_before_nested_finally(a):
    await before()
    try:
        if a:
            try:
                action()
            finally:
                cleanup()
    except Exception:
        recover()
";
        assert_matches_cpython_marshal(source, "terminal_try_condition_ownership.py");
    }
    #[test]
    fn matches_cpython_marshal_for_terminal_try_dynamic_condition_owners() {
        let source = r"def call_then_local():
    try:
        flag = source()
        if flag:
            action()
    except Exception:
        recover()

def unpack_then_local(values):
    try:
        first, second = values
        if first:
            action()
    except Exception:
        recover()

def fast_pair(left, right):
    try:
        if left + right:
            action()
    except Exception:
        recover()

def fast_const(left):
    try:
        if left + 1:
            action()
    except Exception:
        recover()

def global_call():
    try:
        if predicate():
            action()
    except Exception:
        recover()

def local_call(predicate):
    try:
        if predicate():
            action()
    except Exception:
        recover()

def method_call(obj):
    try:
        if obj.predicate():
            action()
    except Exception:
        recover()

def fast_dict(key, value):
    try:
        if {key: value}:
            action()
    except Exception:
        recover()

def constant_dict(value):
    try:
        if {'key': value}:
            action()
    except Exception:
        recover()

def unpack_dict(mapping):
    try:
        if {**mapping}:
            action()
    except Exception:
        recover()

def prior_call_then_while(flag):
    try:
        seed()
        while flag:
            action()
            break
    except Exception:
        recover()
";
        assert_matches_cpython_marshal(source, "terminal_try_dynamic_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_plain_try_condition_marker_owners() {
        let source = r#"def early_return(elements):
    try:
        if not elements:
            return None
        return elements[0]
    except Exception as error:
        return str(error)

def conditional_value(data):
    try:
        value = data["key"] if data else ""
        if not isinstance(value, str):
            raise TypeError
    except (KeyError, TypeError):
        return None
    return value

def comprehension_then_condition(values, flag):
    try:
        items = [value for value in values]
        if flag:
            return items
    except Exception:
        return None

class Derived(Base):
    def specialized_super_then_condition(self, flag):
        try:
            super().prepare()
            if flag:
                return True
        except Exception:
            return False

def adjacent_regions(flag):
    try:
        prepare()
    except ValueError:
        recover()
    try:
        if flag:
            action()
    except Exception:
        recover()

def branch_before_try(branch, flag):
    if branch:
        action()
    try:
        if flag:
            action()
    except Exception:
        recover()
    return 1

def no_default_definition_before_try(flag):
    def inner():
        return 1
    try:
        if flag:
            action()
    except Exception:
        recover()
    return inner

def folded_default_definition_before_try(flag):
    def inner(value=0):
        return value
    try:
        if flag:
            action()
    except Exception:
        recover()
    return inner

def no_default_lambda_before_try(flag):
    inner = lambda: 1
    try:
        if flag:
            action()
    except Exception:
        recover()
    return inner

def folded_default_lambda_before_try(flag):
    inner = lambda value=0: value
    try:
        if flag:
            action()
    except Exception:
        recover()
    return inner

def while_plain_try(flag):
    while flag:
        try:
            if flag:
                action()
        except Exception:
            recover()
        break
"#;
        assert_matches_cpython_marshal(source, "plain_try_condition_marker_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_shifted_plain_try_condition_markers() {
        let source = r"def retained(items):
    for item in items:
        consume(item, 0)
        consume(item, 1)
        consume(item, 2)
        consume(item, 3)
        consume(item, 4)
        consume(item, 5)
        consume(item, 6)
        consume(item, 7)
        try:
            value = item.make()
            if value.flags.owned:
                continue
            value = value.view(Type)
        except Error:
            continue
        use(value)


def shifted(items):
    for item in items:
        consume(item, 0)
        consume(item, 1)
        consume(item, 2)
        consume(item, 3)
        consume(item, 4)
        consume(item, 5)
        consume(item, 6)
        consume(item, 7)
        consume(item, 8)
        try:
            value = item.make()
            if value.flags.owned:
                continue
            value = value.view(Type)
        except Error:
            continue
        use(value)


def direct_condition(items, first, second):
    for item in items:
        consume(first, second)
        consume(first, second)
        try:
            if item:
                continue
            use(item)
        except Error:
            continue
";
        assert_matches_cpython_marshal(source, "shifted_plain_try_condition_markers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_like_terminal_elif_owners() {
        let source = r"def generator_elif(first, second):
    yield 1
    try:
        if first:
            one()
        elif second:
            two()
    except Exception:
        recover()

async def async_generator_elif(first, second):
    yield 1
    try:
        if first:
            one()
        elif second:
            two()
    except Exception:
        recover()

async def coroutine_after_await(first, second, final):
    await before()
    try:
        if first:
            one()
        elif second:
            two()
        if final:
            three()
    except Exception:
        recover()
";
        assert_matches_cpython_marshal(source, "terminal_try_generator_elif.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_none_condition_owners() {
        let source = r"async def post_await_conditions(wait, first, second):
    if first is None:
        before()
    await wait()
    assert first is not None
    if first is None:
        use(first)
    elif second is None:
        use(second)
    if first is None and second is not None:
        use(second)
    return use(first) if second is None else fallback()


async def loop_control(wait, values):
    await wait()
    for value in values:
        if value is None:
            continue


async def try_control(wait, value):
    await wait()
    try:
        if value is None:
            use(value)
    except Error:
        recover()


async def with_control(wait, manager, value):
    await wait()
    with manager:
        if value is None:
            use(value)


async def finally_control(wait, value):
    await wait()
    try:
        use(value)
    finally:
        if value is None:
            cleanup()


def generator_conditions(first, second):
    yield before()
    if first is None and second is not None:
        yield second


def generator_while(value):
    while value is None:
        consume()
    yield value


async def async_generator_after_await(wait, value):
    await wait()
    while value is not None:
        yield value


async def async_generator_after_yield(value):
    yield value
    while value is None:
        consume()
";
        assert_matches_cpython_marshal(source, "generator_none_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_dynamic_specialized_none_condition_owners() {
        let source = r"class GeneratorPrint:
    def _types_to_string(self, value):
        return value

    def generate(self, func):
        log()
        log('Function', bold(func.name))
        with nested():
            desc = func.description
            if '\n' in desc:
                desc = desc[:desc.index('\n')]
            log('Description:', bold(desc))
            log('Return type:', bold(self._types_to_string(func.returns)))
            log('Pos args:', bold(str([x.name for x in func.posargs])))
            log('Opt args:', bold(str([x.name for x in func.optargs])))
            log('Varargs:', bold(func.varargs.name if func.varargs is not None else 'null'))
            log('Kwargs:', bold(str(list(func.kwargs.keys()))))


def verified_loop(matrix):
    norm = lambda values: root(sum(value**2 for value in values))
    previous = None
    precision = starting_precision(matrix)
    epsilon = 2**-precision
    while precision < maximum_precision:
        with working_precision(precision) as context:
            values = context.values(matrix)
            current = norm([part for value in values for part in split(value)])
            if previous is not None and difference(previous, current) < epsilon:
                return values
            previous = current
        precision *= 2
    raise PrecisionError


def clone_shape(checkout, bare, path):
    target = None
    try:
        if not bare:
            target = initialize(path)
            if checkout is None:
                checkout = True
        else:
            target = initialize_bare(path)
        consume(target, checkout)
    except Error:
        recover()


def repeated_bounds(bounds, values):
    try:
        first = transform(values)
        if bounds is not None:
            first = clip(first, bounds)
        second = transform(first)
        if bounds is not None:
            second = clip(second, bounds)
        third = transform(second)
        if bounds is not None:
            third = clip(third, bounds)
        return third
    except Error:
        recover()


def mapping_shape(manager, options, command):
    with manager:
        defaults = {'first': ['a'], 'second': ['b']}
        endings = {'first': [command], 'second': [command, 'tail']}
        if options is not None:
            command.extend(options)
        else:
            command.extend(defaults.get('first', []))
        command.extend(endings['first'])
";
        assert_matches_cpython_marshal(source, "dynamic_none_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_iterator_none_default_owners() {
        let source = r"def protected_defaults(items):
    try:
        for item in items:
            first = item.first
            second = item.second
            if first is None:
                first = 0
            if second is None:
                second = 1
            consume(first, second)
    except Error:
        recover()


def direct_control(value):
    try:
        if value is None:
            value = 0
        consume(value)
    except Error:
        recover()


def with_control(manager, value):
    try:
        with manager:
            if value is None:
                value = 0
            consume(value)
    except Error:
        recover()
";
        assert_matches_cpython_marshal(source, "protected_iterator_none_default_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_tail_assertion_owners() {
        let source = r"async def coroutine_direct(wait, items, value):
    await wait()
    for item in items:
        assert value is None


def generator_or(items, value, other):
    yield value
    for item in items:
        assert value is None or other


async def async_generator_and(items, first, second):
    yield first
    for item in items:
        assert first is None and second


async def protected_try(wait, items, value):
    await wait()
    try:
        for item in items:
            assert value is None
    except Error:
        recover()


async def protected_with(wait, items, value, manager):
    await wait()
    with manager:
        for item in items:
            assert value is None or item


async def protected_finally(wait, items, value):
    try:
        await wait()
    finally:
        for item in items:
            assert value is None
";
        assert_matches_cpython_marshal(source, "loop_tail_assertion_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_exits_across_exception_boundaries() {
        let source = r"try:
    1 / 0
except ZeroDivisionError:
    pass

try:
    raise ValueError
except ValueError as error:
    print(error)

try:
    pass
except Exception:
    pass
finally:
    try:
        pass
    except BaseException as error:
        raise error

try:
    pass
except Exception as error:
    try:
        raise error
    except BaseException:
        pass

try:
    try:
        pass
    except BaseException as error:
        raise error
except Exception:
    pass
";
        assert_matches_cpython_marshal(source, "try_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_small_with_exits() {
        let source = "def normal():\n    with manager:\n        value = 0\n    value = 1\n\ndef terminal():\n    with manager:\n        value = 0\n        return 1\n    value = 1\n\ndef nested_suppression():\n    with first, \\\n        second:\n        raise Error\n    return result\n";
        assert_matches_cpython_marshal(source, "with_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_converted_pop_blocks_in_synchronous_with_bodies() {
        let source = r"def terminal_handler(manager):
    with manager:
        try:
            action()
        except Error:
            raise

def terminal_else(manager):
    with manager:
        try:
            action()
        except Error:
            recover()
        else:
            raise Other

def nested_return(outer, inner):
    with outer:
        with inner:
            return value

def nested_raise(outer, inner):
    with outer:
        with inner:
            raise Error

def no_duplicate_after_finally(manager):
    with manager:
        try:
            action()
        finally:
            cleanup()

def no_duplicate_after_if(manager, condition):
    with manager:
        try:
            action()
            if condition:
                other()
        except Error:
            raise

def multiple_normal_predecessors(manager):
    with manager:
        try:
            action()
        except Error:
            recover()
";
        assert_matches_cpython_marshal(source, "with_converted_pop_block.py");
    }

    #[test]
    fn matches_cpython_marshal_for_handler_fed_pop_blocks_in_synchronous_with_bodies() {
        let source = r"def named(manager):
    with manager:
        try:
            raise Other
        except Error as error:
            recover(error)

def unnamed(manager):
    with manager:
        try:
            raise Error
        except Error:
            recover()

def assertion(manager, condition):
    with manager:
        try:
            raise Other
        except Error:
            assert condition

def body_falls_through(manager):
    with manager:
        try:
            action()
        except Error:
            recover()

def terminal_loop(manager):
    with manager:
        try:
            while True:
                action()
        except Error:
            pass
";
        assert_matches_cpython_marshal(source, "with_handler_fed_pop_block.py");
    }

    #[test]
    fn matches_cpython_marshal_for_handler_returns_inside_synchronous_with() {
        let source = r"def direct(manager):
    with manager:
        try:
            return produce()
        except Error:
            return recover()


def fully_terminating(manager, value, output):
    with manager:
        if is_array(value):
            if output is None:
                return first()
            else:
                return second()
        elif output is None:
            try:
                return typed()
            except AttributeError:
                return fallback()
        else:
            return third()


def enclosing_try_control(manager):
    try:
        with manager:
            return produce()
    except Error:
        return recover()


def fallthrough(manager):
    with manager:
        try:
            produce()
        except Error:
            return False
        return True


def recovery_then_return(manager):
    with manager:
        try:
            produce()
        except Error:
            recover()
            return False
        return True
";
        assert_matches_cpython_marshal(source, "with_direct_handler_returns.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synchronous_with_literal_return_owners() {
        let source = r"def direct(manager):
    try:
        with manager():
            return True
    except Error:
        return False


def arguments(manager, first, second):
    try:
        with manager((first, second), timeout=1.0):
            return True
    except Error:
        return False


def attribute(manager, first, second):
    try:
        with manager.open((first, second), timeout=1.0):
            return True
    except Error:
        return False


def named_control(manager, first, second):
    try:
        with manager.open((first, second), timeout=1.0) as value:
            return value
    except Error:
        return False


def preceding_call(manager, first, second):
    try:
        with manager.open((first, second), timeout=1.0):
            manager.observe()
            return True
    except Error:
        return False
";
        assert_matches_cpython_marshal(source, "synchronous_with_literal_return_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synchronous_with_conditions_after_returns() {
        let source = include_str!("fixtures/synchronous_with_conditions_after_returns.py");
        assert_matches_cpython_marshal(source, "synchronous_with_conditions_after_returns.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synchronous_with_and_conditions_after_returns() {
        let source = include_str!("fixtures/synchronous_with_and_conditions_after_returns.py");
        assert_matches_cpython_marshal(source, "synchronous_with_and_conditions_after_returns.py");
    }

    #[test]
    fn matches_cpython_marshal_for_post_async_with_assignment_condition_owners() {
        let source = include_str!("fixtures/post_async_with_assignment_condition_owners.py");
        assert_matches_cpython_marshal(source, "post_async_with_assignment_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_try_branch_owners() {
        let source = include_str!("fixtures/direct_try_branch_owners.py");
        assert_matches_cpython_marshal(source, "direct_try_branch_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_post_try_sync_with_boolean_value_owners() {
        let source = include_str!("fixtures/post_try_sync_with_boolean_value_owners.py");
        assert_matches_cpython_marshal(source, "post_try_sync_with_boolean_value_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_pre_suspension_coroutine_boolean_value_owners() {
        let source = include_str!("fixtures/pre_suspension_coroutine_boolean_value_owners.py");
        assert_matches_cpython_marshal(source, "pre_suspension_coroutine_boolean_value_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synchronous_with_method_conditions_after_returns() {
        let source = include_str!("fixtures/synchronous_with_method_conditions_after_returns.py");
        assert_matches_cpython_marshal(
            source,
            "synchronous_with_method_conditions_after_returns.py",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_nested_with_terminal_try_exits() {
        let source = r"def else_path(outer):
    with (
        outer() as value,
        value.span,
    ):
        try:
            result = action()
        except Error:
            raise
        else:
            return result


def fallthrough_handler(outer):
    with (
        outer() as value,
        value.span,
    ):
        try:
            action()
        except Error:
            recover()


def same_line_terminal(outer):
    with outer() as value, value.span:
        try:
            result = action()
        except Error:
            raise
        else:
            return result


def direct_return(outer):
    with (
        outer() as value,
        value.span,
    ):
        return action()
";
        assert_matches_cpython_marshal(source, "nested_with_terminal_try_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_multi_with_exit_markers() {
        let source = r"import dis

def three_item_return(first, second, third):
    with (
        first,
        second,
        third,
    ):
        return action()

def three_item_raise(first, second, third):
    with (
        first,
        second,
        third,
    ):
        raise Error

def nested_multi(outer, first, second):
    with outer:
        with (
            first,
            second,
        ):
            return action()

def nested_single(outer, inner):
    with outer:
        with inner:
            return action()

def nonterminal(first, second, third):
    with (
        first,
        second,
        third,
    ):
        action()
    after()

async def asynchronous_scope(first, second, third):
    with (
        first,
        second,
        third,
    ):
        return await action()

def same_line(first, second):
    with first, second:
        return action()

for function in (
    three_item_return,
    three_item_raise,
    nested_multi,
    nested_single,
    nonterminal,
    asynchronous_scope,
    same_line,
):
    print(sum(instruction.opname == 'NOP' for instruction in dis.get_instructions(function)))
";
        assert_executes(source, "1\n1\n1\n1\n0\n1\n0\n");
    }

    #[test]
    fn matches_cpython_marshal_for_trailing_finally_exits_in_synchronous_with_bodies() {
        let source = r"def direct(manager):
    with manager:
        try:
            body()
        finally:
            cleanup()

def followed(manager):
    with manager:
        try:
            body()
        finally:
            cleanup()
    after()

def leading_statement(manager):
    with manager:
        before()
        try:
            body()
        finally:
            cleanup()
    after()

def multiple_items(outer, inner):
    with outer, inner:
        try:
            body()
        finally:
            cleanup()
    after()

def enclosing_try(manager):
    try:
        with manager:
            try:
                body()
            finally:
                cleanup()
        after()
    except Exception:
        recover()

def multiple_finalbody_statements(manager):
    with manager:
        try:
            body()
        finally:
            cleanup_one()
            cleanup_two()
    after()

def nested_managers(outer, inner):
    with outer:
        with inner:
            try:
                body()
            finally:
                cleanup()
    after()

async def synchronous_inside_async(async_manager, sync_manager):
    async with async_manager:
        with sync_manager:
            try:
                await body()
            finally:
                await cleanup()
    await after()

def nontrailing():
    with manager:
        try:
            body()
        finally:
            cleanup()
        inside_after()
    outside_after()

def nested_if(manager, flag):
    with manager:
        if flag:
            try:
                body()
            finally:
                cleanup()
    after()

def nested_loop(manager, items):
    with manager:
        for item in items:
            try:
                body(item)
            finally:
                cleanup(item)
    after()

def unreachable_trailing_try(manager):
    with manager:
        return value()
        try:
            body()
        finally:
            cleanup()

async def asynchronous_manager(manager):
    async with manager:
        try:
            await body()
        finally:
            await cleanup()
    await after()

async def synchronous_outside_async(sync_manager, async_manager):
    with sync_manager:
        async with async_manager:
            try:
                await body()
            finally:
                await cleanup()
    await after()
";
        assert_matches_cpython_marshal(source, "with_finally_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_conditional_finally_exits_in_synchronous_with_bodies() {
        let expression_source = r"def expression_fallthrough(manager, condition):
    with manager:
        try:
            assert body()
        finally:
            if condition:
                cleanup()

def attribute_fallthrough(manager, resource):
    with manager:
        try:
            assert body()
        finally:
            if resource.enabled:
                cleanup()

def call_fallthrough(manager, resource):
    with manager:
        try:
            assert body()
        finally:
            if resource.enabled():
                cleanup()
";
        assert_matches_cpython_marshal(expression_source, "with_conditional_finally_exit.py");

        let direct_termination_source = r"def terminating_body(manager, condition):
    with manager:
        try:
            raise Error()
        finally:
            if condition:
                cleanup()
";
        assert_matches_cpython_marshal(direct_termination_source, "with_finally_direct_raise.py");

        let leading_termination_source = r"def leading_terminating_body(manager, condition):
    with manager:
        before()
        try:
            if condition:
                raise Error()
            raise OtherError()
        finally:
            if condition:
                cleanup()
";
        assert_matches_cpython_marshal(leading_termination_source, "with_finally_leading_raise.py");
    }

    #[test]
    fn matches_cpython_marshal_for_copied_expression_finally_exits() {
        let source = r"def expression_compare(manager, flag):
    with manager:
        try:
            work()
        finally:
            if flag is True:
                cleanup()

def retained_unconditional(manager):
    with manager:
        try:
            work()
        finally:
            cleanup()
";
        assert_matches_cpython_marshal(source, "with_copied_expression_finally.py");
    }

    #[test]
    fn matches_cpython_marshal_for_tail_assert_finally_pop_block_ownership() {
        let source = r"def with_owner(manager, condition):
    with manager:
        try:
            body()
            assert condition
        finally:
            cleanup()


def finally_owner(condition):
    try:
        try:
            body()
            assert condition, message()
        finally:
            inner_cleanup()
    finally:
        outer_cleanup()


def expression_tail(manager):
    with manager:
        try:
            body()
        finally:
            cleanup()
";
        assert_matches_cpython_marshal(source, "tail_assert_finally_pop_block.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nonterminal_iterator_loop_assertion_owners() {
        let source = r"async def coroutine(items):
    for item in items:
        assert item
        consume(item)


async def negated_coroutine(items):
    for item in items:
        assert not item
        consume(item)


def generator(items):
    for item in items:
        assert item
        yield item


async def async_generator(items):
    for item in items:
        assert item
        yield item


async def asynchronous_iterator(items):
    async for item in items:
        assert item
        consume(item)


async def awaited_control(items):
    for item in items:
        assert await predicate(item)
        consume(item)


def ordinary_control(items):
    for item in items:
        assert item
        consume(item)


async def terminal_control(items):
    for item in items:
        consume(item)
        assert item


async def nested_control(items):
    for item in items:
        if predicate(item):
            assert item
            consume(item)
        after(item)


async def while_control(items):
    while items is not None:
        assert items
        consume(items)
        break
";
        assert_matches_cpython_marshal(source, "iterator_loop_assertion_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_first_async_generator_iterator_condition_owners() {
        let source = r"async def first_not_continue(stream):
    async for item in stream:
        if not item:
            continue
        yield item


async def first_attribute_multiple_statements(stream, consume):
    async for item in stream:
        if item.ready:
            consume(item)
        consume(item.value)
        yield item


async def first_call_break(stream, ready):
    async for item in stream:
        if ready(item):
            break
        yield item


async def first_yield_in_branch(stream):
    async for item in stream:
        if item.ready:
            yield item.value
        yield item


async def first_nested_statement(stream, consume):
    async for item in stream:
        if item.ready:
            if item.valid:
                consume(item)
        yield item


async def first_nested_async_for(groups):
    async for group in groups:
        async for item in group:
            if not item:
                continue
            yield item


def plain_for_control(values, consume):
    for item in values:
        if not item:
            continue
        consume(item)


async def coroutine_async_for_control(stream, consume):
    async for item in stream:
        if item.ready:
            consume(item)


def sync_generator_for_control(values):
    for item in values:
        if not item:
            continue
        yield item


async def async_generator_for_control(values):
    for item in values:
        if not item:
            continue
        yield item


async def async_generator_while_control(read):
    while True:
        item = read()
        if not item:
            break
        yield item


async def later_after_assignment_control(stream, consume):
    async for item in stream:
        value = item.value
        if value:
            consume(value)
    yield None


async def later_after_await_control(stream, tick, consume):
    async for item in stream:
        await tick(item)
        if item.ready:
            consume(item)
        yield item


async def later_after_yield_control(stream, consume):
    async for item in stream:
        yield item
        if item.ready:
            consume(item)


async def await_condition_control(stream, ready):
    async for item in stream:
        if await ready(item):
            continue
        yield item


async def generator_condition_control(stream):
    async for item in stream:
        if any(part.ready for part in item.parts):
            continue
        yield item


async def boolean_condition_control(stream, manager, enabled):
    async for item in stream:
        async with manager:
            if enabled and item.ready:
                continue
            yield item


async def conditional_expression_control(stream, manager, enabled):
    async for item in stream:
        async with manager:
            if item.ready if enabled else item.valid:
                continue
            yield item


async def async_with_control(stream, manager):
    async for item in stream:
        async with manager:
            if not item:
                continue
            yield item


def sync_generator_with_control(values, manager):
    with manager:
        for item in values:
            value = item.value
            if not value:
                continue
            yield value


def sync_generator_try_finally_control(values, consume):
    try:
        for item in values:
            value = item.value
            if not value:
                continue
            yield value
    finally:
        consume(values)


async def nested_boolean_async_with_control(stream, manager, enabled, consume):
    async for item in stream:
        async with manager:
            if enabled and item.ready:
                if item.valid or enabled:
                    consume(item)
            yield item
";
        assert_matches_cpython_marshal(source, "async_generator_iterator_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_leading_assignment_iterator_loop_assertion_owners() {
        let source = r"async def coroutine_one(items):
    for item in items:
        state = item.get()
        assert state
        assert state.enabled


async def coroutine_two(items):
    for item in items:
        state = item.get()
        enabled = state.enabled
        assert enabled
        consume(state)


async def coroutine_three(items):
    for item in items:
        state = item.get()
        enabled = state.enabled
        ready = state.ready
        assert ready
        assert enabled


async def rhs_shapes(items):
    for item in items:
        state = item.state
        value = state[0]
        key = f'{value}'
        assert key
        assert state.enabled


async def named_assertion(items):
    for item in items:
        state = item.get()
        assert (result := state.result)
        assert result.enabled


async def attribute_assertion(items):
    for item in items:
        state = item.get()
        assert state.enabled
        assert state.ready


def generator(items):
    for item in items:
        state = item.get()
        assert state
        yield state


async def asynchronous_iterator(items):
    async for item in items:
        state = item.get()
        assert state
        consume(state)


async def awaited_assignment_control(items):
    for item in items:
        state = await item.get()
        assert state
        assert state.enabled


async def async_comprehension_control(items):
    for item in items:
        state = [value async for value in item.values()]
        assert state
        assert item.enabled


async def binary_assignment_control(items):
    for item in items:
        state = item + item
        assert state
        assert state.enabled


async def boolean_call_argument_control(items, fallback):
    for item in items:
        state = item.get(fallback or item)
        assert state
        assert state.enabled


async def comprehension_call_argument_control(items):
    for item in items:
        state = item.get([value for value in item.values if value])
        assert state
        assert state.enabled


async def multiple_target_control(items):
    for item in items:
        state = other = item.get()
        assert state
        assert other.enabled


async def unpack_control(items):
    for item in items:
        state, other = item.get()
        assert state
        assert other.enabled


async def augmented_assignment_control(items, state):
    for item in items:
        state += item.get()
        assert state
        assert item.enabled


async def expression_control(items):
    for item in items:
        consume(item)
        assert item
        assert item.enabled


async def conditional_control(items, enabled):
    for item in items:
        if enabled:
            state = item.get()
        else:
            state = item.other()
        assert state
        assert state.enabled


async def terminal_control(items):
    for item in items:
        state = item.get()
        assert state


def ordinary_control(items):
    for item in items:
        state = item.get()
        assert state
        assert state.enabled
";
        assert_matches_cpython_marshal(source, "leading_assignment_iterator_assertion_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_first_sync_iterator_condition_owner() {
        let source = r#"def two_good_fruits_generator(fruits):
    """A return can be used to end the iterator early, but not a StopIteration."""
    for fruit in fruits:
        if not fruit.is_tasty():
            continue
        while True:
            next_fruit = next(fruits, None)
            if next_fruit is None:
                print("Sorry there is only one fruit left.")
                yield fruit, None
                # We reached the end of the 'fruits' generator but raising a
                # StopIteration instead of returning would create a RuntimeError
                return
            if next_fruit.is_tasty():
                yield fruit, next_fruit
                break
"#;
        assert_matches_cpython_marshal(source, "first_sync_iterator_condition_owner.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_assertions_in_with_bodies() {
        let source = "def simple(manager, condition):\n    with manager:\n        assert condition\n\ndef message(manager, condition):\n    with manager:\n        use()\n        assert condition, make_message()\n\ndef boolean(manager, first, second):\n    with manager:\n        assert first or second\n\ndef conjunction(manager, first, second):\n    with manager:\n        assert first and second\n\ndef conditional(manager, first, second, condition):\n    with manager:\n        assert first if condition else second\n\ndef constant_true():\n    with manager:\n        assert True\n\ndef constant_false():\n    with manager:\n        assert False\n\ndef followed(manager, condition):\n    with manager:\n        assert condition\n    use()\n\ndef same_line(manager, condition):\n    with manager: assert condition\n\ndef nested(outer, inner, condition):\n    try:\n        with outer:\n            with inner:\n                assert condition\n    finally:\n        cleanup()\n\nasync def asynchronous(manager, condition):\n    async with manager:\n        assert condition\n";
        assert_matches_cpython_marshal(source, "with_assert.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_post_suspension_coroutine_assertions() {
        let source = include_str!("fixtures/protected_post_suspension_coroutine_assertions.py");
        assert_matches_cpython_marshal(source, "protected_post_suspension_coroutine_assertions.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_fallthrough_branches_in_with_bodies() {
        let source = r"def simple(manager, first, second):
    with manager:
        if first:
            value = 1
        elif second:
            value = 2
        else:
            value = 3
    return value


def boolean_elif(manager, first, second, third):
    with manager:
        if first:
            value = 1
        elif second and third:
            value = 2
        else:
            value = 3
    return value
";
        assert_matches_cpython_marshal(source, "with_final_fallthrough_branches.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_two_operand_conditions_in_with_bodies() {
        let source = r"def terminal(manager, first, second):
    with manager:
        value = make()
        if not first or not second:
            raise Error(value)


def negated_conjunction(z, msgform):
    with np.errstate(invalid='ignore'):
        if not (np.isnan(z.real) and np.isinf(z.imag)):
            raise AssertionError(msgform % (z.real, z.imag))


def nonterminal(manager, first, second):
    with manager:
        if not first or not second:
            raise Error
        consume()


def fallthrough_body(manager, first, second):
    with manager:
        if not first or not second:
            consume()


def outside_with(first, second):
    if not first or not second:
        raise Error


async def asynchronous(manager, first, second):
    async with manager:
        if not first or not second:
            raise Error
";
        assert_matches_cpython_marshal(source, "with_terminal_two_operand_condition.py");
    }

    #[test]
    fn matches_cpython_marshal_for_with_body_exit_marker_predecessors() {
        let source = r"def multiple_fallthrough_edges(manager, source, expected):
    try:
        with manager:
            values = source()
            if len(values) == 2 and int(values[1]) == expected:
                return True
    except Error:
        pass
    after()


def single_fallthrough_edge(manager, first, second):
    with manager:
        value = make()
        if not first or not second:
            raise Error(value)
";
        assert_matches_cpython_marshal(source, "with_body_exit_marker_predecessors.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_iterator_boolean_continue_owners() {
        let source = r##"def nested_positive(manager, outer, inner, predicates):
    with manager:
        for left in outer:
            left = normalize(left)
            for right in inner:
                right = normalize(right)
                if predicates.ready(left) and predicates.ready(right):
                    continue
                consume(left, right)


def first_positive(manager, lines):
    with manager:
        for line in lines:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            consume(line)


def multiline_condition(manager, values):
    with manager:
        for value in values:
            if (
                ignored(value)
                or hidden(value)
            ):
                continue
            consume(value)


def non_continue(manager, values):
    with manager:
        for value in values:
            if first(value) and second(value):
                consume(value)
            finish(value)


async def asynchronous(manager, values):
    async with manager:
        for value in values:
            if first(value) and second(value):
                continue
            consume(value)
"##;
        assert_matches_cpython_marshal(source, "protected_iterator_boolean_continue.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_iterator_method_continue_owners() {
        let source = r"def prior_assignment(manager, values, make, use):
    with manager:
        for value in values:
            current = make(value)
            if not current.ready():
                continue
            use(current)


def prior_call(manager, values, observe, use):
    with manager:
        for value in values:
            observe(value)
            if not value.ready():
                continue
            use(value)


def nested_for(manager, groups, values, make, use):
    with manager:
        for group in groups:
            for value in values:
                current = make(value)
                if not current.ready():
                    continue
                use(group, current)


def nested_while(manager, groups, values, use):
    with manager:
        while groups:
            for value in values:
                if not value.ready():
                    continue
                use(value)
            groups.pop()


def prior_loop_control(manager, groups, values, use):
    with manager:
        for value in values:
            for group in groups:
                use(group)
            if not value.ready():
                continue
            use(value)


def prior_nested_loop_control(manager, groups, others, values, use):
    with manager:
        for value in values:
            for group in groups:
                for other in others:
                    use(group, other)
            if not value.ready():
                continue
            use(value)


def prior_marker_control(manager, values, use, flag):
    with manager:
        for value in values:
            if flag:
                use(value)
            if not value.ready():
                continue
            use(value)


def body_control(manager, values, observe, use):
    with manager:
        for value in values:
            if not value.ready():
                observe(value)
            use(value)


def break_control(manager, values, use):
    with manager:
        for value in values:
            if not value.ready():
                break
            use(value)


async def async_with_control(manager, values, use):
    async with manager:
        for value in values:
            if not value.ready():
                continue
            use(value)


def try_control(manager, values, use):
    try:
        with manager:
            for value in values:
                if not value.ready():
                    continue
                use(value)
    except Error:
        use(None)
";
        assert_matches_cpython_marshal(source, "sync_with_iterator_method_continue.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_iterator_atomic_continue_owners() {
        let source = r##"def call_condition(manager, lines):
    with manager:
        for line in lines:
            if line.strip().startswith("#"):
                continue
            consume(line)


def name_condition(manager, lines, initial):
    ignoring = initial
    with manager:
        for line in lines:
            if ignoring:
                if line:
                    ignoring = False
                continue
            if line.strip().startswith("#"):
                continue
            consume(line)

"##;
        assert_matches_cpython_marshal(source, "protected_iterator_atomic_continue.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nonfinal_branching_with_body_control() {
        let source = r"def nonfinal(manager, first, second):
    with manager:
        if first:
            value = 1
        elif second:
            value = 2
        else:
            value = 3
        consume(value)
    return value
";
        assert_matches_cpython_marshal(source, "with_nonfinal_branching_control.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_with_returns() {
        let source = "async def value(manager):\n    async with manager as result:\n        return result.value\n\nasync def constant(manager):\n    async with manager:\n        return 1\n\nasync def nested(outer, inner):\n    async with outer:\n        with inner:\n            return value\n";
        assert_matches_cpython_marshal(source, "async_with_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_with_awaited_assignment_exits() {
        let source = r"async def simple(manager):
    async with manager() as value:
        await value()

async def returned(manager):
    async with manager() as value:
        result = await value()
    return result

async def preceded(manager, item):
    async with manager() as value:
        value.prepare(item)
        result = await value()
    return result

async def two_calls(manager, item):
    initial = item.prepare()
    async with manager() as value:
        value.first(initial)
        value.second(initial, flag=True)
        left, right = await value()
    return left

async def tuple_target(manager):
    async with manager() as (first, second):
        result = await second(first)
    return result
";
        assert_matches_cpython_marshal(source, "async_with_awaited_assignment_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_large_protected_async_with_markers() {
        let source = r"async def large_prefix_except(manager, pause, work):
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    try:
        async with manager:
            await pause()
    except Error:
        work()


async def large_body_except_else(manager, pause, work):
    try:
        async with manager:
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            await pause()
    except Error:
        work()
    else:
        work()


async def large_body_finally(manager, pause, work):
    try:
        async with manager:
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            await pause()
    finally:
        work()


async def tuple_assignment_pair(manager, pause, work):
    try:
        async with manager:
            first = work()
            second = await pause()
    except (Error, OtherError):
        work()
    return None


async def tuple_assignment_pair_large_prefix(manager, pause, work):
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    try:
        async with manager:
            first = work()
            second = await pause()
    except (Error, OtherError):
        work()
    return None


async def nested_async_with(manager, nested, pause, work):
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    try:
        async with manager:
            async with nested:
                await pause()
    except Error:
        work()


async def tuple_nonpair_control(manager, pause, work):
    try:
        async with manager:
            await pause()
    except (Error, OtherError):
        work()
    return None
";
        assert_matches_cpython_marshal(source, "large_protected_async_with_markers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_with_exception_exits_in_finally() {
        let source = r"async def tuple_target(manager, close, value):
    try:
        async with manager() as (left, right):
            result = await right(value)
    except Error:
        log()
        raise
    finally:
        await close()
    return result


async def sync_finalizer(manager, close, value):
    try:
        async with manager() as callback:
            result = callback(value)
    except Error:
        raise
    finally:
        close()
    return result


async def handler_pass(manager, close, value):
    try:
        async with manager() as callback:
            result = await callback(value)
    except Error:
        pass
    finally:
        await close()
    return result


async def nested_try(manager, close, value):
    try:
        try:
            async with manager() as callback:
                result = await callback(value)
        except OtherError:
            raise
    except Error:
        raise
    finally:
        await close()
    return result


async def preceded_and_pass_body(manager, close):
    try:
        log()
        async with manager():
            pass
    except Error:
        raise
    finally:
        await close()
    return True


async def no_handler_control(manager, close, value):
    try:
        async with manager() as callback:
            result = await callback(value)
    finally:
        await close()
    return result


async def following_statement_control(manager, close, value):
    try:
        async with manager:
            result = await use(value)
        result = convert(result)
        return result
    except Error:
        raise
    finally:
        await close()


async def nested_manager_control(first, second, close, value):
    try:
        async with first() as left:
            async with second(left) as callback:
                result = await callback(value)
        use(result)
        return result
    except Error:
        raise
    finally:
        await close()


async def multiple_manager_control(first, second, close, value):
    try:
        async with first(), second() as callback:
            result = await callback(value)
        use(result)
        return result
    except Error:
        raise
    finally:
        await close()
";
        assert_matches_cpython_marshal(source, "async_with_exception_exits_in_finally.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_control_unwinding() {
        let source = "def sync_context(manager, items):\n    for item in items:\n        with manager:\n            if item > 0:\n                continue\n            break\n\ndef exception_handler(items):\n    for item in items:\n        try:\n            consume(item)\n        except Exception:\n            handle(item)\n            continue\n        use(item)\n\ntry:\n    for item in items:\n        consume(item)\n        break\nexcept TypeError:\n    handle()\nafter()\n";
        assert_matches_cpython_marshal(source, "loop_control_unwind.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_iterator_cleanup_owners() {
        let source = r"def protected_with_continue(manager, items, magic_key):
    values = set()
    with manager:
        for item in items:
            if not item.startswith(magic_key):
                continue
            pieces = [piece.strip() for piece in item.split(':', 1)]
            if len(pieces) == 2:
                values = values.union(pieces[1].upper().split())
    return values


def protected_iterator_return(items):
    try:
        for item in items:
            if item.ready():
                return item.value().lower()
    except Exception:
        pass
    return None


async def protected_async_iterator_return(items):
    try:
        async for item in items:
            return make(item)
    except Exception:
        pass


def inner_try_return(items):
    for item in items:
        try:
            return item.value()
        except Exception:
            pass


def protected_boolean_return(items):
    try:
        for item in items:
            if item.ready():
                return True
    except Exception:
        pass
    return False


def unprotected_continue(items):
    for item in items:
        if not item.ready():
            continue
        consume(item)
";
        assert_matches_cpython_marshal(source, "protected_iterator_cleanup_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_iterator_value_return_owners() {
        let source = r"def name(rows):
    try:
        for row in rows:
            if row.failed:
                return row
    except RuntimeError:
        return None


def attribute(rows):
    try:
        for row in rows:
            if row.failed:
                return row.status
    except RuntimeError:
        return None


def nested_attribute(rows):
    try:
        for row in rows:
            if row.failed:
                return row.result.status
    except RuntimeError:
        return None


def subscript(rows):
    try:
        for row in rows:
            if row.failed:
                return row['status']
    except RuntimeError:
        return None


def binary(rows):
    try:
        for row in rows:
            if row.failed:
                return row.status + 1
    except RuntimeError:
        return None


def unary(rows):
    try:
        for row in rows:
            if row.failed:
                return -row.status
    except RuntimeError:
        return None


def boolean(rows):
    try:
        for row in rows:
            if row.failed:
                return row.status or 0
    except RuntimeError:
        return None


def unpacked(rows):
    try:
        for _, row in rows:
            if row.failed:
                return row.status
    except RuntimeError:
        return None


def nested_iterator(groups):
    try:
        for rows in groups:
            for row in rows:
                if row.failed:
                    return row.status
    except RuntimeError:
        return None


def nested_try(rows):
    try:
        try:
            for row in rows:
                if row.failed:
                    return row.status
        except ValueError:
            pass
    except RuntimeError:
        return None


def named_handler(rows):
    try:
        for row in rows:
            if row.failed:
                return row.status
    except RuntimeError as error:
        return error


def list_comprehension(rows):
    try:
        for row in rows:
            if row.failed:
                return [item.value for item in row]
    except RuntimeError:
        return None


def dict_comprehension(rows):
    try:
        for row in rows:
            if row.failed:
                return {item.key: item.value for item in row}
    except RuntimeError:
        return None


def set_comprehension(rows):
    try:
        for row in rows:
            if row.failed:
                return {item.value for item in row}
    except RuntimeError:
        return None


def generator_expression(rows):
    try:
        for row in rows:
            if row.failed:
                return (item.value for item in row)
    except RuntimeError:
        return None


def lambda_expression(rows):
    try:
        for row in rows:
            if row.failed:
                return lambda: row.value
    except RuntimeError:
        return None


def named_expression(rows):
    try:
        for row in rows:
            if row.failed:
                return (value := row.value)
    except RuntimeError:
        return None


def yield_expression(rows):
    try:
        for row in rows:
            if row.failed:
                return (yield row.value)
    except RuntimeError:
        return None


async def coroutine_for(rows):
    try:
        for row in rows:
            if row.failed:
                return row.status
    except RuntimeError:
        return None


async def asynchronous_iterator(rows):
    try:
        async for row in rows:
            if row.failed:
                return row.status
    except RuntimeError:
        return None


async def awaited(rows):
    try:
        for row in rows:
            if row.failed:
                return await row.result()
    except RuntimeError:
        return None


async def awaited_async_for(rows):
    try:
        async for row in rows:
            if row.failed:
                return await row.result()
    except RuntimeError:
        return None


def generator_for(rows):
    yield None
    try:
        for row in rows:
            if row.failed:
                return row.status
    except RuntimeError:
        return None


def call_control(rows):
    try:
        for row in rows:
            if row.failed:
                return row.result()
    except RuntimeError:
        return None


def boolean_literal_control(rows):
    try:
        for row in rows:
            if row.failed:
                return False
    except RuntimeError:
        return None


def unprotected_control(rows):
    for row in rows:
        if row.failed:
            return row.status


def inner_try_control(rows):
    for row in rows:
        try:
            if row.failed:
                return row.status
        except RuntimeError:
            pass


def synchronous_manager_control(rows, manager):
    try:
        with manager:
            for row in rows:
                if row.failed:
                    return row.status
    except RuntimeError:
        return None


async def asynchronous_manager_control(rows, manager):
    try:
        async with manager:
            for row in rows:
                if row.failed:
                    return row.status
    except RuntimeError:
        return None
";
        assert_matches_cpython_marshal(source, "protected_iterator_value_return_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_authored_breaks_after_cold_block_ordering() {
        let source = r"def authored_break(items):
    for item in items:
        try:
            action(item)
        except Error:
            recover(item)
            break
        use(item)
    after()


def synthetic_cold_fallthrough(items):
    for item in items:
        try:
            action(item)
        except Error:
            recover(item)
        use(item)
    after()
";
        assert_matches_cpython_marshal(source, "cold_handler_break.py");
    }

    #[test]
    fn matches_cpython_marshal_for_inlined_break_line_nops() {
        let source = r"def conditional(flag):
    while True:
        if flag:
            break
        step()
    return value

def direct():
    while True:
        break
    return value

def same_line():
    while True: break
    return value

def same_line_after_expression():
    while True: step(); break
    return value

def loop_else(flag):
    while flag:
        if done():
            break
        step()
    else:
        fallback()
    return value

def large_exit(flag):
    while True:
        if flag:
            break
        step()
    first(); second(); third(); fourth(); fifth()
    return value

def implicit_return(flag):
    while True:
        if flag:
            break
        step()

def managed(manager, flag):
    while True:
        with manager:
            if flag:
                break
            step()
    return value

def iteration(items, flag):
    for item in items:
        if flag:
            break
    return value
";
        assert_matches_cpython_marshal(source, "inlined_break_nop.py");
    }

    #[test]
    fn matches_cpython_marshal_for_same_line_manager_loop_breaks() {
        let source = r"def empty_read(self, manager):
    with manager as stream:
        while 1:
            item = stream.read()
            if not item: break
    self.finish()


def empty_lines(self, manager):
    with manager as stream:
        while 1:
            items = stream.readlines()
            if items == []: break
    self.finish()
";
        assert_matches_cpython_marshal(source, "same_line_manager_loop_breaks.py");
    }

    #[test]
    fn matches_cpython_marshal_for_finally_loop_break_constant_exits() {
        let source = r"async def coroutine(obj, future):
    try:
        await future
    finally:
        while obj.value > 0:
            if not obj.step():
                break
    return True


def ordinary(obj):
    try:
        work()
    finally:
        while obj.value > 0:
            if not obj.step():
                break
    return True
";
        assert_matches_cpython_marshal(source, "finally_loop_break_constant_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_finally_while_break_owners() {
        let source = r"def initialized_before_try(frame):
    result = 0
    try:
        while frame:
            if ready(frame):
                break
            step(frame)
    finally:
        del frame
    return result


def initialized_inside_try(frame):
    try:
        result = 0
        while frame:
            if ready(frame):
                break
            step(frame)
    finally:
        del frame
    return result


def generator_for_control(items):
    yield value
    try:
        for item in items:
            if ready(item):
                break
            step(item)
    finally:
        del items
    return value


def handler_control(items):
    for item in items:
        try:
            consume(item)
        except Error:
            break
    return value
";
        assert_matches_cpython_marshal(source, "protected_finally_while_break_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_break_after_try_handler() {
        let source = r"def after_handler(items):
    for item in items:
        if item:
            try:
                value = item()
            except Error:
                pass
            break
    return None, None, None


def terminal_branch_before_break(items, first, second):
    for item in items:
        try:
            value = item()
            if first and second:
                return value
        except Error:
            pass
        break
    return None, None, None


def direct_control(items):
    for item in items:
        if item:
            value = item()
            break
    return None, None, None


def named_tuple_control(items):
    first = None
    second = None
    for item in items:
        if item:
            first = item
            break
    return first, second
";
        assert_matches_cpython_marshal(source, "break_after_try_handler.py");
    }

    #[test]
    fn matches_cpython_marshal_for_short_circuit_loop_tail_backedge() {
        let source = r#"def update(target, source, force):
    for key in source:
        if key == "first":
            if force or key not in target:
                target[key] = source[key]
        elif key == "second":
            target[key] = source[key]
        else:
            raise ValueError(key)
"#;
        assert_matches_cpython_marshal(source, "short_circuit_loop_tail_backedge.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_fallthrough_if_exit() {
        let source = r"def nested(flag, inner, extra):
    if flag:
        if inner:
            first()
        else:
            second()
            if extra:
                third()
    else:
        fallback()
    after()
";
        assert_matches_cpython_marshal(source, "nested_fallthrough_if_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multi_exit_nested_if_branch_locations() {
        let source = r"def raised(outer, first, second):
    if outer:
        if first and second:
            raise Error
    else:
        fallback()
    after()

def returned(outer, errors, value):
    if outer:
        if not errors and value is not None:
            return value
    elif value:
        fallback()
    after()

def multiline(outer, first, second):
    if outer:
        if (
            first
            and second
        ):
            raise Error
    else:
        fallback()
    after()

def unique_false_control(outer, first, second):
    if outer:
        if first or second:
            raise Error
    else:
        fallback()
    after()

def simple_control(outer, inner):
    if outer:
        if inner:
            raise Error
    else:
        fallback()
    after()
";
        assert_matches_cpython_marshal(source, "multi_exit_nested_if_branch.py");
    }

    #[test]
    fn matches_cpython_marshal_for_chained_return_exit_sharing() {
        let source = r#"def direct(first, second, third):
    if first <= second <= third:
        return
    after()


def copied_global_exit(first, second, third):
    if first <= second <= third:
        return
    global target
    target = first


def shared_nested_exit(kind, first, second, third):
    if kind == "chain":
        if first <= second <= third:
            return
        global nested_target
        nested_target = first
    else:
        consume()


def identity_chain(xre, xim):
    if xre is xim is None:
        return (None,) * 4
    if xim:
        raise NotImplementedError
    return xre
"#;
        assert_matches_cpython_marshal(source, "chained_return_exit_sharing.py");
    }

    #[test]
    fn matches_cpython_marshal_for_handler_generator_and_try_conditional_owners() {
        let source = r#"def optimized_any_in_handler(matches):
    try:
        result = matches[0]
    except IndexError:
        if any("*" in match for match in matches):
            expanded = []
            for match in matches:
                if "*" in match:
                    try:
                        expanded.append(int(match))
                    except ValueError:
                        expanded.append(match)
                else:
                    expanded.append(match)
            matches = expanded
        result = matches[0]
    return result


def conditional_try_marker(flag, values, index):
    try:
        result = build(values) if flag else values[index]
    except Error:
        recover()
    return result
"#;
        assert_matches_cpython_marshal(source, "handler_generator_try_conditional.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_while_constant_order() {
        let source = r#"async def empty_pool_retained(action, consume):
    while True:
        await action()
        consume(flag=True)

async def prefixed_unused(action):
    "doc"
    while True:
        await action()

async def prefixed_retained(action, consume):
    "doc"
    while True:
        await action()
        consume(flag=True)

async def prefixed_retained_assignment(action):
    "doc"
    while True:
        await action()
        marker = True

async def await_before_loop(action, consume):
    await action()
    while True:
        await action()
        consume(flag=True)
"#;
        assert_matches_cpython_marshal(source, "folded_while_constant_order.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_tail_condition_orientation() {
        let source = r"def terminal_while_break():
    while True:
        value = step()
        if stop(value):
            break
        if condition(value):
            consume(value)

def terminal_while_compound_control():
    while True:
        value = step()
        if stop(value):
            break
        if condition(value):
            consume(value)
        else:
            reject(value)

def nonfinal_nested_while(items):
    for item in items:
        if first(item):
            while condition(item):
                consume(item)
        elif second(item):
            other(item)

def direct_nested_while_control(items):
    for item in items:
        while condition(item):
            consume(item)

def protected_nested_while_control(items):
    try:
        for item in items:
            while condition(item):
                consume(item)
    except Exception:
        recover()

def nonfinal_assertions(items):
    for item in items:
        if first(item):
            assert condition(item)
        else:
            assert other(item)

def nonfinal_break(items):
    for item in items:
        if first(item):
            if stop(item):
                break
        else:
            consume(item)

def nonfinal_raise(items):
    for item in items:
        if first(item):
            if rejected(item):
                raise ValueError(item)
        else:
            consume(item)

def nonfinal_return(items):
    for item in items:
        if first(item):
            if accepted(item):
                return item
        else:
            consume(item)

def direct_assertion_control(items):
    for item in items:
        assert condition(item)
";
        assert_matches_cpython_marshal(source, "loop_tail_condition_orientation.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_loop_tail_first_branch_owners() {
        let source = r"def with_owner(manager, items, match, accept, reject):
    with manager:
        for item in items:
            found = match(item)
            if found:
                if accept(found):
                    accept(item)
                else:
                    reject(found)
            else:
                reject(item)
";
        assert_matches_cpython_marshal(source, "protected_loop_tail_first_branch_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_comprehension_assignment_restores() {
        for source in [
            r"async def f(chunks, obj):
    obj.value = [chunk async for chunk in chunks]
    return obj
",
            r"async def f(chunks):
    value = [chunk async for chunk in chunks]
    return value
",
            r#"async def f(chunks, out):
    out["value"] = [chunk async for chunk in chunks]
    return out
"#,
            r"async def f(chunks):
    chunk = [chunk async for chunk in chunks]
    return chunk
",
            r"async def f(chunks):
    value = (
        [chunk async for chunk in chunks]
    )
    return value
",
            r"async def f(chunks):
    value = [
        chunk async for chunk in chunks
    ]
    return value
",
            r"async def f(chunks):
    a = b = [chunk async for chunk in chunks]
    return a, b
",
            r"async def f(chunks):
    value, = [chunk async for chunk in chunks]
    return value
",
            r"async def f(chunks):
    [value] = [chunk async for chunk in chunks]
    return value
",
            r"async def f(chunks):
    global value
    value = [chunk async for chunk in chunks]
    return value
",
            r"def outer():
    value = None
    async def f(chunks):
        nonlocal value
        value = [chunk async for chunk in chunks]
        return value
    return f
",
            r"async def f(chunks):
    value = [chunk async for chunk in chunks]
    def read():
        return value
    return read
",
            r"async def f(chunks, obj):
    obj.value = {chunk async for chunk in chunks}
    return obj
",
            r"async def f(chunks, obj):
    obj.value = {chunk: chunk async for chunk in chunks}
    return obj
",
            r"async def f(chunks, obj):
    obj.value = [chunk async for chunk in chunks]
    yield obj
",
            r"async def f(chunks, obj):
    try:
        obj.value = [chunk async for chunk in chunks]
    finally:
        await obj.close()
    return obj
",
            r"async def f(chunks):
    a = [a async for a, b in chunks]
    return a
",
            r"async def f(chunks):
    inner = [[inner for inner in chunk] async for chunk in chunks]
    return inner
",
            r"class C:
    async def f(self, chunks):
        __x = [__x async for __x in chunks]
        return __x
",
        ] {
            assert_matches_cpython_marshal(source, "async_comprehension_assignment.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_async_comprehension_cleanup_exits() {
        let source = "async def discarded():\n    [item async for item in source]\n\nasync def returned():\n    return [item async for item in source]\n";
        assert_matches_cpython_marshal(source, "async_comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_filtered_async_comprehension_element_owners() {
        let source = r"async def plain(stream):
    values = [value async for value in stream]
    return values


async def attribute_filter(stream):
    values = [value async for value in stream if value.ready]
    return values


async def method_filter(stream):
    values = [value async for value in stream if value.ready()]
    return values


async def comparison_filter(stream, expected):
    values = [value async for value in stream if value.kind == expected]
    return values


async def conjunction_filter(stream, expected):
    values = [value async for value in stream if value.kind == expected and value.ready]
    return values


async def disjunction_filter(stream, expected):
    values = [value async for value in stream if value.kind == expected or value.ready]
    return values


async def returned(stream):
    return [value async for value in stream if value.ready]


async def set_result(stream):
    values = {value.name async for value in stream if value.ready}
    return values


async def dict_result(stream):
    values = {value.name: value async for value in stream if value.ready}
    return values


async def initialized_target(stream):
    value = None
    values = [value async for value in stream if value.ready]
    return values, value


async def prior_assignment(factory, expected):
    stream = factory()
    values = [value async for value in stream if value.kind == expected]
    return values


async def prior_call(stream, expected, touch):
    touch(stream)
    values = [value async for value in stream if value.kind == expected]
    return values


async def prior_if(stream, expected, gate):
    if gate:
        return []
    values = [value async for value in stream if value.kind == expected]
    return values


async def async_generator(stream, expected):
    values = [value async for value in stream if value.kind == expected]
    for value in values:
        yield value


async def sync_with(stream, expected, manager):
    with manager:
        values = [value async for value in stream if value.kind == expected]
    return values


async def await_iter_control(factory, expected):
    values = [value async for value in await factory() if value.kind == expected]
    return values


async def async_with_control(stream, expected, manager):
    async with manager:
        values = [value async for value in stream if value.kind == expected]
    return values


async def sync_filter_control(values, expected):
    return [value for value in values if value.kind == expected]


async def async_generator_expression_control(stream, expected):
    return (value async for value in stream if value.kind == expected)
";
        assert_matches_cpython_marshal(source, "filtered_async_comprehension_elements.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_async_comprehension_cleanup_order() {
        let source =
            "async def test(): return [[x async for x in elements(n)] async for n in range(3)]\n";
        assert_matches_cpython_marshal(source, "nested_async_comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_assigned_async_comprehension_with_sync_iterable() {
        let source = r"async def positional(values, source):
    result = [item async for item in source([value for value in values])]
    return result

async def keyword(values, source):
    result = [item async for item in source(values=[value for value in values])]
    return result

async def dictionary(values, source):
    result = {item: item async for item in source([value for value in values])}
    return result

async def nested_call(values, source, wrap):
    result = [item async for item in source(wrap([value for value in values]))]
    return result

async def conditional(values, source, enabled):
    result = [item async for item in source([value for value in values] if enabled else [])]
    return result
";
        assert_matches_cpython_marshal(source, "assigned_async_comprehension_sync_iterable.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_expression_child_scope_and_suspension() {
        let cases = [
            (
                "module_direct_generator.py",
                "value = (item for item in items)\n",
            ),
            (
                "module_first_iterable_generator.py",
                "value = [item for item in (inner for inner in items)]\n",
            ),
            (
                "module_nested_generator.py",
                "value = {item: tuple(inner for inner in items) for item in items}\n",
            ),
            (
                "function_direct_generator.py",
                "def f(items):\n    return (item for item in items)\n",
            ),
            (
                "function_first_iterable_generator.py",
                "def f(items):\n    return [item for item in (inner for inner in items)]\n",
            ),
            (
                "function_nested_generator.py",
                "def f(items):\n    return {item: tuple(inner for inner in items) for item in items}\n",
            ),
            (
                "class_direct_generator.py",
                "class Scope:\n    value = (item for item in items)\n",
            ),
            (
                "class_first_iterable_generator.py",
                "class Scope:\n    value = [item for item in (inner for inner in items)]\n",
            ),
            (
                "class_nested_generator.py",
                "class Scope:\n    value = {item: tuple(inner for inner in items) for item in items}\n",
            ),
            (
                "generator_first_iterable_await.py",
                "async def f():\n    return (item for item in await source())\n",
            ),
            (
                "generator_async_for.py",
                "async def f():\n    return (item async for item in source())\n",
            ),
            (
                "generator_awaited_element.py",
                "async def f(items):\n    return (await transform(item) for item in items)\n",
            ),
            (
                "generator_awaited_filter.py",
                "async def f(items):\n    return (item for item in items if await accept(item))\n",
            ),
            (
                "generator_awaited_later_iterable.py",
                "async def f(items):\n    return (value for item in items for value in await expand(item))\n",
            ),
            (
                "generator_inlined_async_element.py",
                "async def f(items):\n    return ([value async for value in expand(item)] for item in items)\n",
            ),
            (
                "generator_nested_scope_boundary.py",
                "async def f(items):\n    return ((await transform(value) for value in expand(item)) for item in items)\n",
            ),
            (
                "generator_later_iterable_inlined_comprehension.py",
                "def f(items, values):\n    return (result for item in items for result in expand(item, [inner for inner in values]))\n",
            ),
            (
                "generator_filter_inlined_comprehension_control.py",
                "def f(items, values):\n    return (item for item in items if accept([inner for inner in values]))\n",
            ),
            (
                "generator_element_inlined_comprehension_control.py",
                "def f(items, values):\n    return ([inner for inner in values] for item in items)\n",
            ),
            (
                "generator_later_iterable_nested_generator_control.py",
                "def f(items, values):\n    return (result for item in items for result in (inner for inner in values))\n",
            ),
            (
                "generator_later_iterable_reused_target_control.py",
                "def f(items, values):\n    return (result for item in items for result in expand([item for item in values]))\n",
            ),
        ];
        for (filename, source) in cases {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_constant_false_match_guard_cleanup() {
        for (pattern, guard) in [
            ("([captured] | [_, captured]) as whole", "False"),
            ("[captured]", "False"),
            ("[head, [tail]]", "False"),
            ("{\"key\": captured}", "False"),
            ("Record(captured)", "False"),
            ("[captured]", "0"),
            ("[captured]", "None"),
            ("0", "False"),
        ] {
            let source = format!(
                "def capture(subject):\n    match subject:\n        case {pattern} if {guard}:\n            pass\n        case _:\n            return subject\n"
            );
            assert_matches_cpython_marshal(&source, "constant_false_match_guard.py");
        }
        for source in [
            "def capture(subject):\n    match subject:\n        case [captured] if False:\n            pass\n        case [captured]:\n            return captured\n        case _:\n            return subject\n",
            "def capture(subject, manager):\n    with manager:\n        match subject:\n            case [captured] if False:\n                pass\n            case _:\n                return subject\n",
        ] {
            assert_matches_cpython_marshal(source, "constant_false_match_guard_context.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_constant_match_guards() {
        let source = "match 1:\n    case _ if (True):\n        pass\n\nmatch 1:\n    case _ if (\n        True\n    ):\n        pass\n";
        assert_matches_cpython_marshal(source, "match_guard.py");
    }

    #[test]
    fn matches_cpython_marshal_for_pass_only_try_else_finally() {
        let source = "try:\n    foo()\nexcept Exception:\n    pass\nelse:\n    pass\nfinally:\n    pass\n\ntry:\n    pass\nexcept Exception:\n    pass\nraise (\n    Exception\n)\n";
        assert_matches_cpython_marshal(source, "try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_trailing_pass_handler_cleanup_locations() {
        let source = r"def typed():
    try:
        body()
    except Error:
        recover()
        pass
    after()

def bare():
    try:
        body()
    except:
        recover()
        pass
    after()

def named():
    try:
        body()
    except Error as error:
        recover(error)
        pass
    after()
";
        assert_matches_cpython_marshal(source, "trailing_pass_handler_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_noop_finally_bodies() {
        let source = r"try:
    body()
except Error:
    recover()
finally:
    ...

try:
    body()
except Error:
    recover()
finally:
    pass
    pass

try:
    try:
        body()
    finally:
        pass
except Error:
    recover()
";
        assert_matches_cpython_marshal(source, "try_finally_noops.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiple_exception_group_handlers() {
        let source = "try:\n    body()\nexcept* ValueError:\n    first()\nexcept* TypeError:\n    second()\n";
        assert_matches_cpython_marshal(source, "try_star.py");
    }

    #[test]
    fn matches_cpython_marshal_for_ownerless_late_exception_dispatch_markers() {
        let source = r"async def coroutine_multiple_handlers():
    await source()
    try:
        action()
    except ValueError:
        first()
    except TypeError:
        second()


def generator_multiple_handlers():
    yield source()
    try:
        action()
    except ValueError:
        first()
    except TypeError:
        second()


def with_multiple_handlers(manager):
    with manager:
        try:
            action()
        except ValueError:
            first()
        except TypeError:
            second()


def finally_multiple_handlers():
    try:
        try:
            action()
        except ValueError:
            first()
        except TypeError:
            second()
    finally:
        cleanup()


async def conditional_exception_type():
    await source()
    try:
        action()
    except (FirstError if condition else SecondError):
        recover()


async def single_handler_control():
    await source()
    try:
        action()
    except ValueError:
        recover()


async def tuple_handler_control():
    await source()
    try:
        action()
    except (ValueError, TypeError):
        recover()


async def bare_fallback_control():
    await source()
    try:
        action()
    except ValueError:
        recover()
    except:
        fallback()
";
        assert_matches_cpython_marshal(source, "handler_dispatch_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_boolean_exception_type_marker_owners() {
        let cases = [
            (
                "handler_boolop_plain.py",
                r"def f():
    try:
        action()
    except A or B or C:
        recover()
",
            ),
            (
                "handler_boolop_generator.py",
                r"def f():
    yield source()
    try:
        action()
    except A and B:
        recover()
",
            ),
            (
                "handler_boolop_with.py",
                r"def f(manager):
    with manager:
        try:
            action()
        except A or B or C:
            recover()
",
            ),
            (
                "handler_boolop_finally.py",
                r"def f():
    try:
        try:
            action()
        except A and B:
            recover()
    finally:
        cleanup()
",
            ),
            (
                "handler_boolop_later.py",
                r"def f():
    try:
        action()
    except X:
        first()
    except A or B or C:
        recover()
",
            ),
            (
                "handler_boolop_dynamic.py",
                r"def f():
    try:
        action()
    except X:
        first()
    except f(g(h())) or B:
        recover()
",
            ),
            (
                "handler_boolop_conditional.py",
                r"def f():
    try:
        action()
    except (A if C else B) or D:
        recover()
",
            ),
            (
                "handler_boolop_comprehension.py",
                r"def f(values):
    try:
        action()
    except [x or y for x in values] or B:
        recover()
",
            ),
            (
                "handler_boolop_exception_group.py",
                r"def f():
    try:
        action()
    except* A or B:
        recover()
",
            ),
        ];

        for (filename, source) in cases {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_a_return_from_an_exception_handler() {
        let source = "def f():\n    value = source()\n    try:\n        raise ValueError\n    except ValueError:\n        return value\n\ndef nested():\n    try:\n        body()\n    except ValueError as error:\n        try:\n            pass\n        except TypeError:\n            raise error\n\ndef nested_message():\n    try:\n        body()\n    except ValueError as error:\n        try:\n            finish()\n        except TypeError:\n            log(f\"error: {error}\")\n";
        assert_matches_cpython_marshal(source, "handler_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_try_return_swap_owners() {
        let source = r"def target(var):
    try:
        return var['kindselector']['*']
    except KeyError:
        try:
            return var['kindselector']['kind']
        except KeyError:
            pass

def direct(var):
    try:
        return var['kindselector']['*']
    except KeyError:
        return var['kindselector']['kind']

def named(var):
    try:
        return var['kindselector']['*']
    except KeyError as error:
        try:
            return var['kindselector']['kind']
        except KeyError:
            pass

def literal(var):
    try:
        action()
    except KeyError:
        try:
            return False
        except TypeError:
            pass
";
        assert_matches_cpython_marshal(source, "nested_try_return_swap_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_exception_cleanup() {
        let source = r"class Container:
    def nested_handler(self):
        try:
            body()
        except ValueError as __error:
            try:
                pass
            except TypeError:
                raise __error
";
        assert_matches_cpython_marshal(source, "named_exception_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_return_cleanup() {
        let source = r"class Container:
    def return_from_handler(self):
        try:
            body()
        except Error as __error:
            return __error
";
        assert_matches_cpython_marshal(source, "named_handler_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nonterminal_handler_condition_fallthrough() {
        let source = r"def named_raise(flag):
    try:
        action()
    except Error as error:
        if flag:
            raise error
    after()


def named_return(flag):
    try:
        action()
    except Error as error:
        if flag:
            return error
    after()


def bare_raise(flag):
    try:
        action()
    except Error:
        if flag:
            raise
    after()


def final_elif(first, second):
    try:
        action()
    except Error as error:
        if first:
            raise error
        elif second:
            raise error
    after()


def explicit_else(flag):
    try:
        action()
    except Error as error:
        if flag:
            raise error
        else:
            consume(error)
    after()
";
        assert_matches_cpython_marshal(source, "nonterminal_handler_fallthrough.py");
    }

    #[test]
    fn matches_cpython_marshal_for_trailing_if_handler_cleanup() {
        let source = r"def unknown_name_test(flag):
    try:
        action()
    except Error:
        if flag:
            recover()
    after()


def compare_test(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        if key not in mapping.values():
            print(key)
    value = key


def named_call_body(mapping, key):
    try:
        value = mapping[key]
    except KeyError as error:
        if key not in mapping.values():
            print(error)
    value = key


def bare_call_body(mapping, key):
    try:
        value = mapping[key]
    except:
        if key not in mapping.values():
            print(key)
    value = key


def explicit_else_control(flag):
    try:
        action()
    except Error as error:
        if flag:
            consume(error)
        else:
            finish()


def direct_tail(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        print(key)
    value = key


def noncall_control(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        if key not in mapping.values():
            raise
    value = key


def multiple_statement_control(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        prepare()
        if key not in mapping.values():
            print(key)
        finish()
    value = key


def nontrailing_control(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        if key not in mapping.values():
            print(key)
        finish()
    value = key
";
        assert_matches_cpython_marshal(source, "trailing_if_handler_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_inherited_trailing_if_handler_cleanup() {
        let source = r"def inherited_from_if(flag, mapping, key):
    if flag:
        try:
            value = mapping[key]
        except KeyError:
            if key not in mapping.values():
                value = key
    after()


def inherited_through_nested_if(outer, inner, mapping, key):
    if outer:
        if inner:
            try:
                value = mapping[key]
            except KeyError as error:
                if key not in mapping.values():
                    value = error
    after()


def inherited_from_with(manager, mapping, key):
    with manager:
        try:
            value = mapping[key]
        except KeyError:
            if key not in mapping.values():
                value = key
    after()


async def inherited_from_async_with(manager, mapping, key):
    async with manager:
        try:
            value = mapping[key]
        except KeyError:
            if condition(key):
                recover(key)
    await after()
";
        assert_matches_cpython_pop_except_positions(
            source,
            "inherited_trailing_if_handler_cleanup.py",
        );
    }

    #[test]
    fn preserves_located_handler_cleanup_across_loop_barriers() {
        let source = r"def for_barrier(keys, mapping):
    for key in keys:
        try:
            value = mapping[key]
        except KeyError:
            if key not in mapping.values():
                value = key
    after()


def nested_for_barrier(flag, manager, keys, mapping):
    if flag:
        with manager:
            for key in keys:
                try:
                    value = mapping[key]
                except KeyError:
                    if key not in mapping.values():
                        value = key
    after()
";
        for (qualified_name, line) in [("for_barrier", 7), ("nested_for_barrier", 19)] {
            assert_cpython_and_serc_have_pop_except_at(
                source,
                "inherited_handler_loop_barriers.py",
                qualified_name,
                line,
            );
        }
    }

    #[test]
    fn matches_cpython_marshal_for_multi_exit_handler_cleanup_boundaries() {
        let cases = [
            (
                r"def following_continuation(var):
    try:
        return 1 / var.some_other_func()
    except AttributeError:
        if (message := var.get_message()):
            pass
    except ZeroDivisionError:
        message = 'division by zero'
    print(message)
",
                "following_handler_cleanup.py",
            ),
            (
                r"def loop_tail_control(value, items):
    for item in items:
        try:
            action(item)
        except:
            if True:
                pass
    consume(value)
",
                "loop_tail_handler_cleanup.py",
            ),
            (
                r"def compound_arm_control(value, items):
    try:
        pass
    except:
        if True:
            for item in items:
                pass
    value = None
",
                "compound_handler_cleanup.py",
            ),
            (
                r"def assert_only_control(value):
    try:
        pass
    except:
        if True:
            assert False
    value = None
",
                "assert_handler_cleanup.py",
            ),
        ];

        for (source, filename) in cases {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_none_condition_marker_owners_in_exception_handlers() {
        let source = r"def is_none(value):
    try:
        action()
    except Error:
        if value is None:
            recover()
        finish()


def is_not_none(value, items):
    try:
        action()
    except Error:
        if value is not None:
            recover()
        result = [item for item in items]


def named(value):
    try:
        action()
    except Error as error:
        if value is None:
            recover(error)
        finish()


def nested(value):
    try:
        action()
    except Error:
        if predicate(value):
            if value is not None:
                recover()
        finish()


def reversed_control(value):
    try:
        action()
    except Error:
        if None is value:
            recover()
        finish()


def outside_handler_control(value):
    if value is None:
        recover()
    finish()


def later_bare_handler_control(value):
    try:
        action()
    except ValueError as error:
        consume(error)
    except TypeError:
        if value is None:
            recover()
        finish()
";
        assert_matches_cpython_marshal(source, "handler_none_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_nested_finally_cleanup() {
        let source = r"class Container:
    def fallthrough_from_nested_try(self):
        try:
            body()
        except Error as __error:
            try:
                consume(__error)
            finally:
                pass
";
        assert_matches_cpython_marshal(source, "named_handler_nested_finally.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_nested_try_else_cleanup() {
        let source = r"def nested_branch(flag, header):
    try:
        value = parse(header)
    except SyntaxError as error:
        if flag:
            try:
                value = parse_legacy(header)
            except SyntaxError as nested:
                raise ValueError(header) from nested
            else:
                warn(header)
        else:
            raise ValueError(header) from error
    consume(value)
    return value


def direct_control(header):
    try:
        value = parse(header)
    except SyntaxError as error:
        try:
            value = parse_legacy(header)
        except SyntaxError as nested:
            raise ValueError(header) from nested
        else:
            warn(header)
    consume(value)
    return value
";
        assert_matches_cpython_marshal(source, "named_handler_nested_try_else.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_return_finally_copies() {
        let source = r"def nested_conditional_cleanup(path, left, right):
    try:
        try:
            return left == right
        finally:
            if path.exists():
                path.unlink()
    finally:
        release()


def three_nested_cleanups(value):
    try:
        try:
            try:
                return value
            finally:
                inner()
        finally:
            middle()
    finally:
        outer()


def single_cleanup_control(value):
    try:
        return value
    finally:
        cleanup()
";
        assert_matches_cpython_marshal(source, "nested_return_finally.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_try_else_exit_locations() {
        let source = r#"def terminal_try_else(flag):
    try:
        action()
    except Error:
        pass
    else:
        if flag:
            return produce()
    return ""


def nonterminal_try_else_control(flag):
    try:
        action()
    except Error:
        pass
    else:
        if flag:
            consume()
    return ""


def try_body_control(flag):
    try:
        if flag:
            return produce()
    except Error:
        pass
    return ""
"#;
        assert_matches_cpython_marshal(source, "try_else_terminal_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_trailing_with_cleanup() {
        let source = r"def named_handler_trailing_with(outer, inner):
    with outer:
        try:
            action()
        except Error as error:
            with inner:
                recover(error)
        else:
            finish()
";
        assert_matches_cpython_marshal(source, "named_handler_trailing_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_handler_cleanup_before_terminating_else() {
        let source = r#"class VisualCompiler:
    if platform:
        try:
            compiler = make_compiler()
            compiler.initialize()
        except AttributeError as error:
            if "root" in str(error):
                report(error)
            else:
                raise
        except OSError as error:
            if not expected(error):
                report(error)
                raise

    executable = compiler


def terminating_body_control(flag):
    try:
        action()
    except Error as error:
        if flag:
            raise
        else:
            recover(error)
    after()


def outside_handler_control(flag):
    if flag:
        recover()
    else:
        raise Error
    after()
"#;
        assert_matches_cpython_marshal(source, "handler_cleanup_before_else.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_inlineable_cleanup() {
        let source = r"def trailing_try(key):
    try:
        value = first[key]
    except KeyError:
        try:
            value = second[key]
        except KeyError:
            raise NameError(key) from None
    return value


def trailing_if(flag):
    try:
        action()
    except Error:
        value = None
        if flag:
            value = recover()
        else:
            value = fallback()
    return value


def trailing_if_with_expression_join(flag):
    try:
        action()
    except Error:
        value = None
        if flag:
            value = (
                first()
                or second()
            )
        else:
            value = fallback()
    return value


def trailing_if_with_terminating_arm(first, second):
    try:
        action()
    except Error:
        if first:
            raise
        elif second:
            value = recover()
        else:
            value = fallback()
    return value


def nested_handler_return_before_following(first, second):
    try:
        value = first()
    except Error:
        try:
            result = second()
        except OtherError:
            return None
        def normalize(value):
            return value
        return normalize(result)
    return value
";
        assert_matches_cpython_marshal(source, "bare_handler_inlineable_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_nested_try_normal_exits() {
        let source = r"def fallback_import():
    try:
        import primary
        state = 2
    except ImportError:
        try:
            import fallback
            state = 1
        except ImportError as error:
            report(error)
            state = 0
    consume(state)


def nested_try_with_following_handler_statement():
    try:
        value = first()
    except LookupError:
        try:
            value = second()
        except LookupError:
            value = fallback()
        consume(value)
    return value


def terminating_nested_handler(key):
    try:
        value = first[key]
    except KeyError:
        try:
            value = second[key]
        except KeyError:
            raise NameError(key) from None
    return value
";
        assert_matches_cpython_marshal(source, "bare_handler_nested_try_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_trailing_fallthrough_try_cleanup() {
        let source = r"def trailing_fallthrough_try(key):
    try:
        value = primary[key]
    except KeyError:
        if key:
            value = fallback[key]
        try:
            value = secondary[key]
        except KeyError:
            report(key)
    return value


def following_handler_statement_control(key):
    try:
        value = primary[key]
    except KeyError:
        try:
            value = secondary[key]
        except KeyError:
            report(key)
        consume(value)
    return value


def nested_handler_branch_control(key):
    try:
        value = primary[key]
    except KeyError:
        if key:
            try:
                value = secondary[key]
            except KeyError:
                report(key)
        consume(value)
    return value
";
        assert_matches_cpython_marshal(source, "bare_handler_fallthrough_try_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_conversion_loop_handler_exits() {
        let source = r"def classify(values):
    result = []
    for value in values:
        try:
            int(value)
        except (ValueError, TypeError):
            try:
                float(value)
            except (ValueError, TypeError):
                try:
                    complex(value)
                except (ValueError, TypeError):
                    result.append(type(value))
                else:
                    result.append(complex)
            else:
                result.append(float)
        else:
            result.append(int)
    return result


def two_levels(values):
    result = []
    for value in values:
        try:
            int(value)
        except (ValueError, TypeError):
            try:
                float(value)
            except (ValueError, TypeError):
                result.append(type(value))
            else:
                result.append(float)
        else:
            result.append(int)
    return result


def unlocated_bare_raise_cleanup(attribute, fields, options):
    try:
        assign(attribute)
    except Exception:
        fields = fields or {}
        options = options or {}
        if not (attribute in fields or attribute in options):
            raise
    finish()
";
        assert_matches_cpython_marshal(source, "nested_conversion_loop_handler_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_try_body_branch_exit_owners() {
        let source = r"def direct_exit(marker, text):
    try:
        if marker in text:
            left, right = first()
        else:
            value = second()
            left, right = value.real, value.imag
    except Error:
        raise ValueError
    consume(left, right)


def direct_comparison_exit(array, dtype):
    try:
        if array.shape[1] > 0:
            consolidated = array.view(dtype)
        else:
            consolidated = empty(len(array), dtype=dtype)
    except TypeError as error:
        message = 'unsupported dtype'
        raise TypeError(message) from error
    consume(consolidated)


def direct_assignment_fallthrough(flag):
    try:
        if flag:
            value = first()
        else:
            value = second()
    except Error:
        value = fallback()
    consume(value)


def protected_join_control(marker, text):
    try:
        if marker in text:
            left, right = first()
        else:
            value = second()
            left, right = value.real, value.imag
        consume(left, right)
    except Error:
        raise ValueError


def try_else_control(flag):
    try:
        if flag:
            first()
        else:
            second()
    except Error:
        report()
    else:
        success()
    after()
";
        assert_matches_cpython_marshal(source, "direct_try_body_branch_exit_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_nested_try_exception_owners() {
        let source = r"def terminal_nested_try(flag, items):
    try:
        if flag:
            try:
                for item in items:
                    consume(item)
            except Exception as error:
                report(error)
                report_done()
    except OuterError:
        recover()
    finish()


def nonterminal_nested_try(flag, items):
    try:
        if flag:
            try:
                for item in items:
                    consume(item)
            except Exception as error:
                report(error)
                report_done()
        finish_inside()
    except OuterError:
        recover()
    finish()
";
        assert_matches_cpython_marshal(source, "terminal_nested_try_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_cleanup_controls() {
        let source = r"def implicit_fallthrough(flag):
    try:
        action()
    except Error:
        if flag:
            raise
    after()


def single_normal_exit(flag):
    try:
        action()
    except Error:
        if flag:
            raise
        else:
            recover()
    after()
";
        assert_matches_cpython_marshal(source, "bare_handler_cleanup_controls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_handler_loop_and_cleanup_owners() {
        let source = r#"def bare_handler_loop_marker(values):
    try:
        action()
    except Error:
        for value in values:
            match = regex.search("value", value)
            if match:
                result = match.group("value")
            if result is not None:
                break
        return result


def ordinary_bare_handler_condition(value):
    try:
        action()
    except Error:
        if regex.search("value", value):
            recover()
        finish()


def protected_break_and_following_empty_handler(values):
    result = []
    try:
        for value in values:
            if value == "stop":
                break
            result.append(value)
        try:
            result.append(source.attribute())
        except AttributeError:
            pass
        consume(
            value00, value01, value02, value03, value04,
            value05, value06, value07, value08, value09,
            value10, value11, value12, value13, value14,
            value15, value16, value17, value18, value19,
            value20, value21, value22, value23, value24,
            value25, value26, value27, value28, value29,
            value30, value31, value32, value33, value34,
            value35, value36, value37, value38, value39,
        )
    except ImportError:
        pass
    return result


def triple_named_handlers_with_continuation():
    try:
        first()
    except Error as error:
        first_message = str(error)
        try:
            second()
        except Error as error:
            second_message = str(error)
            try:
                third()
                result = 3
            except Error as error:
                third_message = str(error)
                consume(first_message, second_message, third_message)
    normalized = normalize(result)
    consume(normalized)
    return normalized
"#;
        assert_matches_cpython_marshal(source, "nested_handler_cleanup_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_call_cleanup_inside_with() {
        let source = r"def protected(manager, flag):
    with manager:
        try:
            action()
        except Error:
            result = check()
            if result:
                report(result)


def explicit_else_control(flag):
    try:
        action()
    except Error:
        first()
        if flag:
            second()
        else:
            third()
";
        assert_matches_cpython_marshal(source, "bare_handler_call_cleanup.py");
    }

    #[test]
    fn preserves_enclosing_finally_owner_for_empty_nested_try() {
        for (filename, source) in [
            (
                "empty_nested_try_sync.py",
                "def f(y):\n    try:\n        try:\n            pass\n        except:\n            pass\n    finally:\n        y()\n",
            ),
            (
                "empty_nested_try_async.py",
                "async def f(y):\n    try:\n        try:\n            pass\n        except:\n            pass\n    finally:\n        await y()\n",
            ),
            (
                "empty_nested_try_generator.py",
                "def f(y):\n    try:\n        try:\n            pass\n        except:\n            pass\n    finally:\n        y()\n    yield\n",
            ),
            (
                "empty_nested_try_typed.py",
                "def f(y):\n    try:\n        try:\n            pass\n        except ValueError:\n            pass\n    finally:\n        y()\n",
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn trace_error_runs_enclosing_finally_for_empty_nested_try() {
        let source = r"import sys

def f(y):
    try:
        try:
            pass
        except:
            pass
    finally:
        y()

events = []
def cleanup():
    events.append('cleanup')
def trace(frame, event, arg):
    if frame.f_code is f.__code__ and event == 'line' and frame.f_lineno == f.__code__.co_firstlineno + 3:
        raise ValueError('trace failed')
    return trace
sys.settrace(trace)
try:
    f(cleanup)
except ValueError:
    events.append('raised')
finally:
    sys.settrace(None)
print(events)
";
        assert_executes(source, "['cleanup', 'raised']\n");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_expressions_in_protected_regions() {
        let source = "def try_except():\n    try:\n        'try'\n        body()\n    except:\n        'except'\n        recover()\n\ndef except_only():\n    try:\n        body()\n    except:\n        'except'\n\ndef nested_handlers():\n    try:\n        try:\n            body()\n        except ValueError:\n            recover()\n        except TypeError:\n            recover()\n    finally:\n        cleanup()\n\ndef with_body(manager):\n    with manager:\n        'with'\n        body()\n\ndef with_only(manager):\n    with manager:\n        'with'\n\nasync def protected_condition():\n    await source\n    if test:\n        await other\n\ntry:\n    pass\nexcept (ValueError if True else TypeError):\n    pass\n\ntry:\n    pass\nexcept* (ValueError if True else TypeError):\n    pass\n\ndef protected_conditions(manager, first, second):\n    with manager:\n        if first:\n            body()\n        if second:\n            body()\n\ndef generator_try():\n    yield 1\n    try:\n        pass\n    except Exception:\n        pass\n\nif condition:\n    try:\n        with manager:\n            body()\n    except Error:\n        pass\n";
        assert_matches_cpython_marshal(source, "protected_nop.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_large_integer_bitwise_operations() {
        let source = "bit_and = 99999999999999999999999 & 0o200000\nbit_or = 99999999999999999999999 | 0o777\nbit_xor = 18446744073709551616 ^ 18446744073709551616\nsum = 9999999999999999999999999999999999999999 + 9999999999999999999999999999999999999999\n";
        assert_matches_cpython_marshal(source, "large_bitwise.py");
    }

    #[test]
    fn matches_cpython_marshal_for_negative_large_integer_literals() {
        let source = "minimum_i64 = -9223372036854775808\nbelow_minimum_i64 = -9223372036854775809\nnegative_u64_max = -18446744073709551615\nnegative_u64_overflow = -18446744073709551616\n";
        assert_matches_cpython_marshal(source, "negative_large_integers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_negative_true_division() {
        let source = "fraction = -5 / 7\ntenth = -1 / 10\nlarge_denominator = -1 / 9_000_000_000_000_000_000\ndivision_by_zero = -1 / 0\n";
        assert_matches_cpython_marshal(source, "negative_true_division.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_branches_in_a_with_statement() {
        let source = "from contextlib import suppress\n\ndef f():\n    with suppress(Exception):\n        if 1:\n            pass\n        elif 1:\n            pass\n        elif 1:\n            pass\n        try:\n            pass\n        except Exception:\n            pass\n        finally:\n            pass\n        if 2:\n            pass\n        while True:\n            pass\n";
        assert_matches_cpython_marshal(source, "protected_branches.py");
    }

    #[test]
    fn matches_cpython_marshal_for_coroutine_finally_condition() {
        let source = "async def f():\n    try:\n        await source\n    finally:\n        if test:\n            await other\n";
        assert_matches_cpython_marshal(source, "finally_condition.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_boolean_operands() {
        let source = "if a and f() and False and g():\n    pass\na or \"\" or True\na or () or True\na and \"value\" and False\na or (b or c) or d\n0 if a or [1] or True or [2] else 1\nif (\n    f()\n    is None\n):\n    pass\nif g() is not None:\n    pass\nx\n\ndef terminal_conditional_expression(wait):\n    call() if wait else None\n\nif first or low <= value < high:\n    pass\n\nmultiline = [\n    (\n        \"value\"\n        and condition\n        and result\n    )\n]\n";
        assert_matches_cpython_marshal(source, "bool.py");
    }

    #[test]
    fn matches_cpython_marshal_for_trailing_nested_boolean_locations() {
        let source = r"def conjunction(first, second, third):
    return first and (second and third)


def disjunction(first, second, third):
    return first or (second or third)


strip = lambda value: ((b'x' in value and value.strip()) or
                       ((b'x' not in value and value.lower()) or None))
";
        assert_matches_cpython_marshal(source, "trailing_nested_boolean_locations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unique_condition_fallthrough_locations() {
        let source = r#"def terminal_try(left, right):
    try:
        assert left and right
    except TypeError:
        pass


def terminal_branch(flag, left, right):
    if flag:
        assert left and right
    else:
        consume()


def protected(value):
    with manager(invalid="ignore"):
        result = transform(value)
        if result.real != 0 or result.imag != 0:
            raise AssertionError(format_result(result.real, result.imag))


def nested_join(flag, left, right):
    if flag:
        if not (left and right):
            raise ValueError
    elif alternate:
        consume()


def nested_loop(items, left, right):
    for item in items:
        while left or right:
            left, right = update(item)
"#;
        assert_matches_cpython_marshal(source, "unique_condition_fallthrough_locations.py");
    }

    #[test]
    fn matches_cpython_3145_boolean_exception_ranges() {
        let source = "async def choose(value):\n    return value or 'empty'\n\ntry:\n    body()\nexcept FirstError or SecondError as error:\n    recover(error)\n\ntry:\n    body()\nexcept FirstError and SecondError as error:\n    recover(error)\n";
        assert_matches_cpython_marshal(source, "boolean_exception_ranges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_normalized_comparison_marker_owners() {
        let source = r"async def coroutine():
    await source
    if test == expected:
        await other


def generator(value):
    while value > 0:
        yield value
        value -= 1
";
        assert_matches_cpython_marshal(source, "normalized_comparison_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_genexpr_chained_comparison_marker_slots() {
        let source = r"def slot_zero_small_int(items):
    return (0 < item < 10 for item in items)

def slot_zero_float(items):
    return (0.0 < item < 10 for item in items)

def slot_one_null_global(items):
    return (lower() < item < upper for item in items)

def slot_one_fused_store(items):
    return (0 < left < right for left, right in items)
";
        assert_matches_cpython_marshal(source, "genexpr_chained_comparison_marker_slots.py");
    }

    #[test]
    fn matches_cpython_marshal_for_boolean_value_marker_owners() {
        let source = "def generator_expression(items):\n    return (item or fallback for item in items)\n\ndef generator_value(value):\n    yield value\n    result = value and fallback\n    consume(result)\n\ndef generator_try(value):\n    yield value\n    try:\n        result = value or fallback\n        consume(result)\n    finally:\n        cleanup()\n\ndef ordinary_try(value):\n    try:\n        result = value or fallback\n        consume(result)\n    finally:\n        cleanup()\n\ndef nested_try(value):\n    try:\n        try:\n            result = value and fallback\n            consume(result)\n        finally:\n            inner_cleanup()\n    finally:\n        outer_cleanup()\n\ndef with_only(manager, value):\n    with manager:\n        result = value or fallback\n        consume(result)\n\ndef try_inside_with(manager, value):\n    with manager:\n        try:\n            result = value and fallback\n            consume(result)\n        finally:\n            cleanup()\n\ndef with_inside_try(manager, value):\n    try:\n        with manager:\n            result = value or fallback\n            consume(result)\n    finally:\n        cleanup()\n\ndef comprehension(items):\n    return [item and fallback for item in items]\n\ndef protected_comprehensions(manager, items):\n    try:\n        left = [item or fallback for item in items]\n        with manager:\n            right = {item and fallback for item in items}\n        return left, right\n    finally:\n        cleanup()\n\ndef generator_comprehension(items):\n    yield [item or fallback for item in items]\n\nasync def coroutine_before(value):\n    result = value or fallback\n    await source\n    consume(result)\n\nasync def coroutine_after(value):\n    await source\n    result = value and fallback\n    consume(result)\n\nasync def coroutine_try(value):\n    await source\n    try:\n        result = value or fallback\n        consume(result)\n    finally:\n        cleanup()\n\nasync def coroutine_with(manager, value):\n    await source\n    with manager:\n        result = value and fallback\n        consume(result)\n\nasync def async_generator(value):\n    yield value\n    result = value or fallback\n    consume(result)\n\nasync def async_generator_try(value):\n    yield value\n    try:\n        result = value and fallback\n        consume(result)\n    finally:\n        cleanup()\n";
        assert_matches_cpython_marshal(source, "boolean_value_marker_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_comprehension_boolean_stale_owners() {
        let source = "def unowned(items):\n    return [int(item or 0) for item in items]\n\ndef owned(items):\n    return [(int(timestamp), int(count or 0)) for count, timestamp in items]\n";
        assert_matches_cpython_marshal(source, "comprehension_boolean_stale_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_boolean_value_owner_edges() {
        let source = include_str!("fixtures/boolean_value_owner_edges.py");
        assert_matches_cpython_marshal(source, "boolean_value_owner_edges.py");
    }

    #[test]
    fn preserves_try_star_boolean_value_marker_owners() {
        let source = include_str!("fixtures/try_star_boolean_value_owners.py");
        assert_executes(source, "True\nTrue\n");
    }

    #[test]
    fn matches_cpython_marshal_for_path_sensitive_boolean_value_owners() {
        let source = include_str!("fixtures/boolean_value_owner_paths.py");
        assert_matches_cpython_marshal(source, "boolean_value_owner_paths.py");
    }

    #[test]
    fn matches_cpython_marshal_for_backward_boolean_assert_jumps() {
        let source = "def f(items):\n    for item in items:\n        assert (\"x\" in item) or (item in values), \"bad\"\n";
        assert_matches_cpython_marshal(source, "backward_bool.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_elif_short_circuit_loop_tails() {
        let source = r"def direct_and(items, first, second):
    for item in items:
        if item == 0:
            first()
        elif item and second:
            second()


def direct_not_or(items, first, second):
    while items:
        item = items.pop()
        if item == 0:
            first()
        elif not (item or second):
            second()


async def direct_async(items, first, second):
    async for item in items:
        if item == 0:
            first()
        elif item and second:
            second()


def nested_loop(items, first, second):
    for item in items:
        if item:
            for nested in item:
                if first:
                    first()
                elif nested and second:
                    second()
        elif second:
            second()
";
        assert_matches_cpython_marshal(source, "final_elif_short_circuit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_non_final_branch_nested_loop_tails() {
        let source = r#"def for_loop(items):
    for item in items:
        if classify(item) == "first":
            if item:
                consume(item)
        elif classify(item) == "second":
            if item is not None:
                consume(item)
        else:
            fallback(item)


def while_loop(items):
    while items:
        item = items.pop()
        if is_first(item):
            if item:
                consume(item)
        elif is_second(item):
            consume(item)


def recursively_nested(items):
    for item in items:
        if is_first(item):
            if item:
                if accepted(item):
                    consume(item)
        elif is_second(item):
            consume(item)


def final_elif_in_non_final_branch(items):
    for item in items:
        if is_first(item):
            if accepted(item):
                consume(item)
            elif deferred(item):
                if ready(item):
                    consume(item)
        else:
            fallback(item)
"#;
        assert_matches_cpython_marshal(source, "non_final_branch_loop_tail.py");
    }

    #[test]
    fn matches_cpython_marshal_for_copied_non_final_iterator_branch_exits() {
        let source = r"def atomic(items):
    for item in items:
        if first(item):
            if accepted(item):
                consume(item)
        elif second(item):
            fallback(item)


def conjunction(items):
    for item in items:
        if first(item):
            if accepted(item) and ready(item):
                consume(item)
        elif second(item):
            fallback(item)


def disjunction(items):
    for item in items:
        if first(item):
            if accepted(item) or ready(item):
                consume(item)
        elif second(item):
            fallback(item)


def direct_loop_control(items):
    for item in items:
        if accepted(item) and ready(item):
            consume(item)


def with_control(manager, items, match, accept, reject):
    with manager:
        for item in items:
            found = match(item)
            if found:
                if accept(found):
                    accept(item)
                else:
                    reject(found)
            else:
                reject(item)


def try_control(items):
    try:
        for item in items:
            while condition(item):
                consume(item)
    except Exception:
        recover()


def while_control(items):
    for item in items:
        while condition(item):
            consume(item)


def assert_control(items):
    for item in items:
        assert condition(item)
";
        assert_matches_cpython_marshal(source, "copied_non_final_iterator_branch_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_inherited_non_final_branch_loop_tails() {
        let source = r"def append_library(libraries, item):
    for existing in libraries:
        if is_sequence(existing):
            if is_sequence(item):
                if item[0] == existing[0]:
                    if item[1] is existing[1]:
                        return
                    warn(item[0])
                    break
            else:
                if item == existing[0]:
                    warn(item[0])
                    break
        else:
            if is_sequence(item):
                if item[0] == existing:
                    warn(item[0])
                    break
            else:
                if item == existing:
                    return
    libraries.append(item)


def final_else_control(items):
    for item in items:
        if is_sequence(item):
            consume(item)
        else:
            if accepted(item):
                consume(item)
";
        assert_matches_cpython_marshal(source, "inherited_non_final_branch_loop_tails.py");
    }

    #[test]
    fn uses_cpython_non_final_short_circuit_assignment_orientation() {
        let source = r#"import dis


def short_circuit_assignment(items, force, accepted, replacement):
    for item in items:
        if is_first(item):
            if force or item not in accepted:
                accepted[item] = replacement[item]
        else:
            fallback(item)


line = short_circuit_assignment.__code__.co_firstlineno + 3
print(" ".join(
    instruction.opname
    for instruction in dis.get_instructions(short_circuit_assignment)
    if instruction.positions.lineno == line
    and (
        instruction.opname.startswith("POP_JUMP")
        or instruction.opname == "JUMP_BACKWARD"
    )
))
"#;
        assert_executes(source, "POP_JUMP_IF_TRUE POP_JUMP_IF_FALSE JUMP_BACKWARD\n");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_and_async_iteration_edges() {
        let source = fixtures::LOOP_ASYNC_EDGES;
        assert_matches_cpython_marshal(source, "loop_tail.py");
    }

    #[test]
    fn matches_cpython_marshal_for_post_yield_empty_block_borrow_boundaries() {
        let source = r"def nested_try_exit(value):
    try:
        action()
    except Error:
        recover()
    else:
        begin()
        try:
            yield
        finally:
            finish()
    consume(value)
    return value


async def simple_async_break(items, value):
    async for item in items:
        break
    consume(value)
    return value


async def tail_conditional_async_break(items, value):
    async for item in items:
        if item:
            break
    consume(value)
    return value
";
        assert_matches_cpython_marshal(source, "post_yield_borrow_edges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_constant_false_assertions_after_if_joins() {
        let source = r"def after_if_pass(flag, message):
    if flag:
        pass
    assert False, message


def after_if_return(flag, message):
    if flag:
        return
    assert False, message


def after_if_raise(flag, error, message):
    if flag:
        raise error
    assert False, message


def after_if_else(flag, message):
    if flag:
        first()
    else:
        second()
    assert False, message


def after_if_chain(first, second, message):
    if first:
        one()
    elif second:
        two()
    assert False, message


def assert_alone(message):
    assert False, message


def after_expression(message):
    consume()
    assert False, message


def after_assignment(message):
    value = produce()
    assert False, message


def dynamic_after_if(flag, condition, message):
    if flag:
        consume()
    assert condition, message
";
        assert_matches_cpython_marshal(source, "assert_after_if_join.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_async_for_in_with_bodies() {
        let source = "async def sync_with_direct(manager, items):\n    with manager:\n        async for item in items:\n            consume(item)\n\nasync def async_with_direct(manager, items):\n    async with manager:\n        async for item in items:\n            consume(item)\n\nasync def sync_then_async_with(sync_manager, async_manager, items):\n    with sync_manager:\n        async with async_manager:\n            async for item in items:\n                consume(item)\n\nasync def async_then_sync_with(async_manager, sync_manager, items):\n    async with async_manager:\n        with sync_manager:\n            async for item in items:\n                consume(item)\n\nasync def async_with_two_items(first, second, items):\n    async with first, second:\n        async for item in items:\n            consume(item)\n\nasync def async_with_inner_finally(manager, items):\n    async with manager:\n        try:\n            async for item in items:\n                consume(item)\n        finally:\n            cleanup()\n\nasync def async_with_nonfinal(manager, items):\n    async with manager:\n        async for item in items:\n            consume(item)\n        after()\n";
        assert_matches_cpython_marshal(source, "async_with_terminal_async_for.py");

        let source = "async def try_around_async_with(manager, items):\n    try:\n        async with manager:\n            async for item in items:\n                consume(item)\n    finally:\n        cleanup()\n";
        assert_matches_cpython_marshal(source, "try_finally_async_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_for_noop_body_backedges() {
        let source = "async def pass_body(items):\n    async for item in items:\n        pass\n\nasync def two_passes(items):\n    async for item in items:\n        pass\n        pass\n\nasync def docstring_and_pass(items):\n    async for item in items:\n        'docstring'\n        pass\n\nasync def nested(items):\n    async for outer in items:\n        async for inner in outer:\n            pass\n\nasync def with_else(items):\n    async for item in items:\n        pass\n    else:\n        pass\n";
        assert_matches_cpython_marshal(source, "async_for_noop_body.py");
    }

    #[test]
    fn matches_cpython_terminal_async_for_nop_ownership() {
        let source = "import dis\n\nasync def sync_with_direct(manager, items):\n    with manager:\n        async for item in items:\n            consume(item)\n\nasync def async_then_sync_with(async_manager, sync_manager, items):\n    async with async_manager:\n        with sync_manager:\n            async for item in items:\n                consume(item)\n\nasync def try_around_async_with(manager, items):\n    try:\n        async with manager:\n            async for item in items:\n                consume(item)\n    finally:\n        cleanup()\n\nasync def async_with_around_try(manager, items):\n    async with manager:\n        try:\n            async for item in items:\n                consume(item)\n        finally:\n            cleanup()\n\ndef has_end_nop(function):\n    instructions = list(dis.get_instructions(function))\n    return any(\n        left.opname == 'END_ASYNC_FOR' and right.opname == 'NOP'\n        for left, right in zip(instructions, instructions[1:])\n    )\n\nfor function in (sync_with_direct, async_then_sync_with, try_around_async_with, async_with_around_try):\n    print(has_end_nop(function))\n";
        assert_executes(source, "True\nTrue\nFalse\nTrue\n");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_boolean_comprehension() {
        let source = "foo or {x: None for x in bar}\n";
        assert_matches_cpython_marshal(source, "boolean_comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_short_circuit_comprehension_filters() {
        let source = "z = [a for a in range(5) if a or b or c or d and e]\n";
        assert_matches_cpython_marshal(source, "comprehension_filter.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_none_comprehension_filters() {
        let source = r"def generator_name(items):
    return (
        item
        for item in items
        if (
            item
            is not None
        )
    )

def generator_named(items):
    return (
        value
        for item in items
        if (
            value := transform(item)
        ) is not None
    )

def list_call(items):
    return [
        item
        for item in items
        if (
            transform(item)
            is None
        )
    ]

def set_named(items):
    return {
        value
        for item in items
        if (
            value := transform(item)
        ) is not None
    }

def dict_named(items):
    return {
        value: item
        for item in items
        if (
            value := transform(item)
        ) is not None
    }

def same_line_generator(items):
    return (item for item in items if item is not None)

def same_line_list(items):
    return [item for item in items if transform(item) is None]
";
        assert_matches_cpython_marshal(source, "multiline_none_comprehension_filters.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_comprehension_filter_exits() {
        let source = r"def with_owner(manager, items):
    with manager:
        values = [item.name for item in items if item.is_file()]
    return values

def none_filter_owner(manager, items):
    with manager:
        present = [item for item in items if item is not None]
    return present
";
        assert_matches_cpython_marshal(source, "protected_comprehension_filter_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unprotected_comprehension_filter_exits() {
        let source = r"def unprotected(items):
    return [item for item in items if keep(item)]

def generator_scope(manager, items):
    with manager:
        values = (item for item in items if keep(item))
    return values
";
        assert_matches_cpython_marshal(source, "unprotected_comprehension_filter_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_imported_attribute_calls_in_module_comprehensions() {
        let source = r#"import re
import re as captured_module
import re as inner_only_module
import re as lambda_default_module
import re as lambda_module
import re as later_target_module
import re as outer_lambda_module
import re as outer_target_module
import re as target_module
import re as target_expression_module
import re as walrus_module

ordinary = re.match("a", "a")
first_iterable = [item for item in re.split(",", "a,b")]
filtered = [item for item in ("a", "b") if re.match("a", item)]
elements = [re.split(",", item) for item in ("a,b", "c,d")]
dictionary = {item: re.compile(item) for item in ("a", "b")}
unique = {re.escape(item) for item in ("a", "b")}
nested = [
    re.match("a", inner)
    for outer in (1,)
    for inner in re.split(",", "a,b")
]
generator = (re.match("a", item) for item in ("a", "b"))

target_shadow = [
    target_module.match("a", "a")
    for target_module in (target_module,)
]
later_target_shadow = [
    later_target_module.match("a", "a")
    for item in (1,)
    for later_target_module in (later_target_module,)
]
target_expression = [
    None
    for target_expression_module.match("a", "a").target in (1,)
]
captured_target = [
    (lambda: captured_module.match("a", "a"))
    for captured_module in (captured_module,)
]
walrus_shadow = [
    walrus_module.match("a", item)
    for item in ("a",)
    if (walrus_module := walrus_module)
]
inner_only_values = (inner_only_module,)
inner_target_only = [
    [
        inner_only_module.match("a", "a")
        for inner_only_module in inner_only_values
    ]
    for outer in (1,)
]
outer_target_inner_free = [
    [outer_target_module.match("a", "a") for inner in (1,)]
    for outer_target_module in (outer_target_module,)
]
lambda_only = [
    (lambda: lambda_module.match("a", "a"))
    for item in (1,)
]
lambda_default = [
    (lambda value=lambda_default_module.match("a", "a"): value)
    for item in (1,)
]
outer_free_and_lambda = [
    (
        outer_lambda_module.match("a", "a"),
        lambda: outer_lambda_module.match("a", "a"),
    )
    for item in (1,)
]

def function_scope():
    return [re.match("a", item) for item in ("a", "b")]
"#;
        assert_matches_cpython_marshal(source, "imported_attribute_comprehensions.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_comprehension_global_lookups() {
        let source = r"GLOBAL = 10
groups = ((1, 2),)

def decorate(value):
    return lambda function: function

class Example:
    source = (1, 2, 3)
    callbacks = (str,)
    values = [GLOBAL + value for value in source]
    labels = {str(value) for value in source if bool(value)}
    mapping = {value: tuple((GLOBAL, value)) for value in source}
    nested = [[str(inner) for inner in range(outer)] for outer in source]
    called = [callback(value) for callback in callbacks for value in source]
    shadowed = [GLOBAL for GLOBAL in source]

def enclosing(captured):
    class Nested:
        values = [GLOBAL + captured + value for value in range(3)]

        def method(self):
            return __class__, captured

    return Nested

class Decorated:
    @decorate([item for item in group] for group in groups)
    def method(self):
        return None
";
        assert_matches_cpython_marshal(source, "class_comprehension_globals.py");
    }

    #[test]
    fn matches_cpython_marshal_for_annotation_comprehension_cells() {
        for (filename, source) in [
            (
                "function_annotation_comprehension_cells.py",
                r"def function(
    sequence: [lambda: item for item in (1,)],
    unique: {lambda: item for item in (2,)},
) -> {key: lambda: key for key in (3,)}:
    pass

def outer(values):
    def inner(argument: [lambda: item for item in values]):
        pass
    return inner
",
            ),
            (
                "module_annotation_comprehension_cells.py",
                r"sequence: [lambda: item for item in (1,)]
if True:
    mapping: {key: lambda: key for key in (2,)}
nested: [[lambda: (outer, inner) for inner in (3,)] for outer in (4,)]
",
            ),
            (
                "future_annotation_comprehension_cells.py",
                r"from __future__ import annotations
sequence: [lambda: item for item in (1,)]
def function(argument: [lambda: item for item in (2,)]):
    pass
",
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn executes_annotation_comprehension_cells() {
        let source = r"def function(argument: [lambda: item for item in (1, 2)]):
    pass
sequence: [lambda: item for item in (3, 4)]
print([callback() for callback in function.__annotations__['argument']])
print([callback() for callback in __annotate__(1)['sequence']])
";
        assert_executes(source, "[2, 2]\n[4, 4]\n");
    }

    #[test]
    fn matches_cpython_marshal_for_conditional_class_annotations() {
        let source = r"flag = True

class Conditional:
    direct: str
    if flag:
        enabled: int
    else:
        disabled: bytes

class WithClassCell:
    if flag:
        member: __class__

    def method(self):
        return __class__
";
        assert_matches_cpython_marshal(source, "conditional_class_annotations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_large_annotation_indices() {
        let mut source = String::from("flag = True\n");
        for index in 0..=256 {
            writeln!(&mut source, "module_{index}: int").unwrap();
        }
        source.push_str("\nclass Conditional:\n    if flag:\n");
        for index in 0..=256 {
            writeln!(&mut source, "        class_{index}: int").unwrap();
        }

        // Index 255 remains a LOAD_SMALL_INT; index 256 becomes a LOAD_CONST. Exercise the
        // transition in both the executable marker and deferred annotation thunk for modules
        // and conditional class annotations.
        assert_matches_cpython_marshal(&source, "large_annotation_indices.py");
    }

    #[test]
    fn matches_cpython_marshal_for_future_conditional_class_annotation_cells() {
        let source = r"from __future__ import annotations

flag = True

class Conditional:
    if flag:
        enabled: int
    else:
        disabled: bytes = b''

class NestedConditional:
    if flag:
        while flag:
            nested: str

class Unconditional:
    direct: int

class ConditionalWithoutAnnotation:
    if flag:
        direct = 1
";
        assert_matches_cpython_marshal(source, "future_conditional_class_annotation_cells.py");
    }

    #[test]
    fn matches_cpython_marshal_for_negated_generator_filters() {
        let source =
            "def filtered(items):\n    return (item for item in items if not predicate(item))\n";
        assert_matches_cpython_marshal(source, "negated_generator_filter.py");
    }

    #[test]
    fn matches_cpython_marshal_for_comparison_filter_cleanup() {
        let source = r"def generator_disjunction(items, left, right):
    return (item for item in items if item == left or item == right)

def generator_chained(items, lower, upper, excluded):
    return (
        item
        for item in items
        if lower < item < upper and item != excluded
    )

def generator_ranges(items, first, lower, upper, last):
    return (
        item
        for item in items
        if item == first or lower <= item <= upper or item == last
    )

def negated_inlined(items, marker, other):
    return [item for item in items if not (item is marker and item == other)]

def negated_generator(items, marker, other):
    return (item for item in items if not (item is marker and item == other))

";
        assert_matches_cpython_marshal(source, "comparison_filter_cleanup.py");
    }

    #[test]
    fn matches_cpython_marshal_for_applied_comprehension_cleanup_swaps() {
        let source = r"def applied(manager, condition, items):
    with manager:
        result = [item for item in items]
        if condition:
            return result

def discarded(manager, condition, items):
    with manager:
        {item for item in items}
        if condition:
            return

def retained(manager, condition, items):
    with manager:
        result = consume([item for item in items])
        if condition:
            return result
";
        assert_matches_cpython_marshal(source, "comprehension_cleanup_swap.py");
    }

    #[test]
    fn matches_cpython_marshal_for_maybe_null_comprehension_restores() {
        let source = r#"def assigned(items):
    target = "outer"
    [target for target in items]
    first(target)
    second(target)

def parameter(target, items):
    [target for target in items]
    first(target)
    second(target)

def with_target(manager, items):
    with manager as target:
        [target for target in items]
        first(target)
        second(target)

def unrelated(items):
    value = "outer"
    [target for target in items]
    first(value)
    second(value)

def rebound(items):
    target = "outer"
    [target for target in items]
    target = "new"
    first(target)
    second(target)

def discarded(items):
    target = "outer"
    {target for target in items}
    first(target)
    second(target)
"#;
        assert_matches_cpython_marshal(source, "maybe_null_comprehension_restore.py");
    }

    #[test]
    fn leaves_comprehension_cell_restores_definitely_initialized() {
        let source = r#"import dis

def cell_restore(items):
    target = "outer"
    [target for target in items]
    return (target for _ in items)

for instruction in dis.get_instructions(cell_restore):
    if instruction.argval == "target" and instruction.opname.startswith("LOAD_FAST"):
        print(instruction.opname)
"#;
        assert_executes(source, "LOAD_FAST_AND_CLEAR\nLOAD_FAST_BORROW\n");
    }

    #[test]
    fn matches_cpython_marshal_for_inlined_comprehension_cells() {
        let source = r"module_result = [lambda: x for x in range(2)]

def captured_targets():
    lists = [lambda: x for x in range(2)]
    sets = {lambda: x for x in range(2)}
    dictionaries = {x: lambda: x for x in range(2)}
    return lists, sets, dictionaries

def unreachable_target():
    if False:
        return [lambda: item for item in range(3)]

def nested_targets(xs, ys):
    return [[lambda: x for x in ys] for x in xs]

def target_rebound_as_later_cell(types, values):
    normalized = {normalize(type_) for type_ in types}
    for type_ in types:
        converted = (convert(value, type_) for value in values)
    return normalized, converted
";
        assert_matches_cpython_marshal(source, "comprehension_cells.py");
    }

    #[test]
    fn matches_cpython_marshal_for_class_scope_super_resolution() {
        let source = r"class Outer:
    def method(self):
        class Inner:
            super(Outer, self).method()

            def nested(inner_self):
                super(Outer, self).method()


class Annotated:
    def method(self):
        __class__: object
        super


class Shadowed:
    super = factory
    value = super(Shadowed, instance).method()
";
        assert_matches_cpython_marshal(source, "class_super.py");
    }

    #[test]
    fn matches_cpython_marshal_for_captured_implicit_super_receivers() {
        let source = r"class Base:
    def method(self):
        pass


class Child(Base):
    def captured_self(self):
        def capture():
            return self
        return super().method()

    @classmethod
    def captured_cls(cls):
        def capture():
            return cls
        return super().method()

    def ordinary(self):
        return super().method()

    def explicit(self):
        def capture():
            return self
        return super(Child, self).method()
";
        assert_matches_cpython_marshal(source, "captured_super.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_super_method_arguments() {
        let source = "class Base:\n    pass\n\nclass Child(Base):\n    def implicit(self, value):\n        return super().method(\n            value\n        )\n\n    def explicit(self, value):\n        return super(Child, self).method(\n            value\n        )\n\n    def keyword(self, value):\n        return super().method(\n            option=value,\n        )\n\n    def split_super(self, value):\n        return super(\n        ).method(\n            value,\n        )\n\n    def multiline_attribute(self):\n        return super().\\\n            value\n";
        assert_matches_cpython_marshal(source, "super_method_lines.py");
    }

    #[test]
    fn matches_cpython_marshal_for_large_call_thresholds() {
        let keywords = |count| {
            (0..count)
                .map(|index| format!("k{index}={index}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let positional = |count| {
            (0..count)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let keywords_15 = keywords(15);
        let keywords_16 = keywords(16);
        let keywords_28 = keywords(28);
        let keywords_29 = keywords(29);
        let positional_27 = positional(27);
        let positional_28 = positional(28);
        let positional_31 = positional(31);
        let source = format!(
            r"import imported

def method_sixteen(obj):
    return obj.method({keywords_16})

def global_sixteen():
    return target({keywords_16})

def global_one_and_fifteen():
    return target(0, {keywords_15})

def global_thirty_one():
    return target({positional_31})

def imported_one_and_fifteen():
    return imported.target(0, {keywords_15})

class Base:
    def method(self):
        pass

class Child(Base):
    def super_sixteen(self):
        return super().method({keywords_16})

    def super_twenty_eight_and_one(self):
        return super().method({positional_28}, option=0)

def method_twenty_eight(obj):
    return obj.method({keywords_28})

def method_twenty_nine(obj):
    return obj.method({keywords_29})

def method_twenty_seven_and_one(obj):
    return obj.method({positional_27}, option=0)

def method_twenty_eight_and_one(obj):
    return obj.method({positional_28}, option=0)
"
        );
        assert_matches_cpython_marshal(&source, "large_call_thresholds.py");
    }

    #[test]
    fn matches_cpython_marshal_for_starred_global_call_arguments() {
        let source = r"global module_values, module_callable
module_explicit_global = consume(*module_values)
module_load_name = consume(*module_name)
module_both_global = module_callable(*module_values)
module_multiline = consume(
    *module_values
)
module_named_keyword = consume(*module_values, option=True)

module_direct_global = module_callable()

class ClassScope:
    global class_values, class_callable
    explicit_global = consume(*class_values)
    load_name = consume(*class_name)
    both_global = class_callable(*class_values)
    named_keyword = consume(*class_values, option=True)

    direct_global = class_callable()


class AnnotationScope:
    type Alias = consume(*annotation_values)


def global_callable():
    return consume(*values)

def local_callable(consume):
    return consume(*values)

def local_argument(values):
    return consume(*values)

def bound_callable(obj):
    return obj.consume(*values)

def computed_callable():
    return make_consumer()(*values)

def comprehension_shadow():
    global values
    return [consume(*values) for values in collections]

def called_argument():
    return consume(*make_values())

def leading_argument():
    return consume(1, *values)

def multiple_arguments():
    return consume(*values, *more_values)

def keyword_unpacking():
    return consume(*values, **options)
";
        assert_matches_cpython_marshal(source, "starred_global_argument.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_generator_calls() {
        let source = "module_any = any(value for value in values)\nmodule_all = all(value for value in values)\nmodule_tuple = tuple(value for value in values)\nany(value for value in values)\nall(value for value in values)\ntuple(value for value in values)\nafter = None\n\n\ndef optimized(values):\n    return (\n        any(value for value in values),\n        all(value for value in values),\n        tuple(value for value in values),\n    )\n\n\ndef rebound(values, any):\n    return any(value for value in values)\n\n\ndef terminal_discard(values):\n    all(value for value in values)\n";
        assert_matches_cpython_marshal(source, "optimized_generator_calls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_generator_calls_in_try_bodies() {
        let source = r"def direct_nonterminal(values):
    try:
        result = tuple(int(value) for value in values)
        if result:
            consume(result)
    except ValueError:
        recover()
";
        assert_matches_cpython_marshal(source, "optimized_generator_try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_boolean_generators_in_protected_bodies() {
        let source = r"def try_assignment(values, touch):
    try:
        result = any(value.ready for value in values)
        touch(result)
    except Error:
        pass

def try_condition(values, touch):
    try:
        if any(value.ready for value in values):
            touch(values)
    except Error:
        pass

def try_boolean_conditions(values, state, touch):
    try:
        if state.active and any(value.ready for value in values):
            touch(state)
        if state.active or all(value.ready for value in values):
            touch(state)
    except Error:
        pass

def try_finally(values, touch, cleanup):
    try:
        if any(value.ready for value in values):
            touch(values)
    finally:
        cleanup()

def try_loop(groups, touch):
    try:
        for values in groups:
            result = any(value.ready for value in values)
            touch(result)
    except Error:
        pass

def with_direct(manager, values, touch):
    with manager:
        result = any(value.ready for value in values)
        touch(result)

def condition_before_with(manager, values, flag, touch):
    if flag:
        touch(flag)
    with manager:
        result = any(value.ready for value in values)
        touch(result)

def with_boolean_assignment(manager, values, state, touch):
    with manager:
        result = state.active and any(value.ready for value in values)
        touch(result)

def with_boolean_condition(manager, values, state, touch):
    with manager:
        if state.active or all(value.ready for value in values):
            touch(state)

def with_prior_condition(manager, values, state, touch):
    with manager:
        if state.active:
            touch(state)
        result = any(value.ready for value in values)
        touch(result)

def with_loop(manager, groups, touch):
    with manager:
        for values in groups:
            result = any(value.ready for value in values)
            touch(result)
";
        assert_matches_cpython_marshal(source, "optimized_boolean_protected.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_coroutine_boolean_generators() {
        let source = include_str!("fixtures/protected_coroutine_boolean_generators.py");
        assert_matches_cpython_marshal(source, "protected_coroutine_boolean_generators.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synchronous_with_short_circuit_none_markers() {
        let source = r"def two_operands(manager, value, touch):
    with manager:
        if value.ready and value is not None:
            touch(value)

def three_operands(manager, value, touch):
    with manager:
        if value.ready and value is not None and value.active:
            touch(value)

def nested_body(manager, value, fallback, touch):
    with manager:
        if value.ready and value is not None and value.active:
            if fallback(value):
                touch(value)

def none_last(manager, value, fallback, touch):
    with manager:
        if value.ready and value.active and value is not None:
            if fallback(value):
                touch(value)

def disjunction(manager, value, fallback, touch):
    with manager:
        if value.ready or value is None or value.active:
            if fallback(value):
                touch(value)

def prior_condition(manager, value, flag, fallback, touch):
    with manager:
        if flag:
            touch(value)
        if value.ready and value is not None and value.active:
            if fallback(value):
                touch(value)

def following_statement(manager, value, touch):
    with manager:
        if value.ready and value is not None and value.active:
            touch(value)
        touch(value)

def nested_manager(first, second, value, fallback, touch):
    with first:
        with second:
            if value.ready and value is not None and value.active:
                if fallback(value):
                    touch(value)

def try_around_with(manager, value, fallback, touch):
    try:
        with manager:
            if value.ready and value is not None and value.active:
                if fallback(value):
                    touch(value)
    except Error:
        pass

def none_first_control(manager, value, touch):
    with manager:
        if value is not None and value.ready and value.active:
            touch(value)

def loop_control(manager, values, touch):
    with manager:
        for value in values:
            if value.ready and value is not None and value.active:
                touch(value)

def nested_try_control(manager, value, touch):
    with manager:
        try:
            if value.ready and value is not None and value.active:
                touch(value)
        except Error:
            pass
";
        assert_matches_cpython_marshal(source, "sync_with_short_circuit_none.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_guard_marker_owners() {
        let source = r"def after_resume(values):
    yield before()
    result = tuple(value for value in values)
    yield result


def while_fresh(values):
    yield before()
    while True:
        result = tuple(value for value in values)
        yield result


def after_dynamic(flag, values):
    yield before()
    if flag:
        prepare()
        result = tuple(value for value in values)
        yield result


def iterator_owned(groups):
    yield before()
    for values in groups:
        result = tuple(value for value in values)
        yield result


def consecutive_fresh(values):
    yield before()
    while True:
        first = tuple(value for value in values)
        second = tuple(value for value in values)
        yield first, second
";
        assert_matches_cpython_marshal(source, "generator_guard_marker_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_boolean_optimized_generator_calls() {
        let source = "def disjunction(prefix, values):\n    return prefix or any(value for value in values)\n\ndef conjunction(prefix, values):\n    return prefix and all(value for value in values)\n\ndef nested(first, second, values):\n    return first or second or any(value for value in values)\n\nwrapped = lambda first, second, values: first and second and all(value for value in values)\n";
        assert_matches_cpython_marshal(source, "boolean_generator_calls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_boolean_generator_dispatch_owners() {
        let source = r"def assert_all_all(values):
    assert all(all(item.ready for item in row) for row in values)


def assert_all_any(values):
    assert all(any(item.ready for item in row) for row in values)


def assert_any_all(values):
    assert any(all(item.ready for item in row) for row in values)


def assert_any_any(values):
    assert any(any(item.ready for item in row) for row in values)


def returned_nested_all(values):
    return all(all(item.ready for item in row) for row in values)


def returned_nested_any(values):
    return any(any(item.ready for item in row) for row in values)


def assigned_nested_all_any(values):
    result = all(any(item.ready for item in row) for row in values)
    return result


def assigned_nested_any_all(values):
    result = any(all(item.ready for item in row) for row in values)
    return result


async def coroutine_nested_all_any(values):
    return all(any(item.ready for item in row) for row in values)


async def coroutine_nested_any_all(values):
    return any(all(item.ready for item in row) for row in values)


def generator_nested_all(values):
    yield all(all(item.ready for item in row) for row in values)


async def async_generator_nested_any(values):
    yield any(any(item.ready for item in row) for row in values)


def try_protected_nested(values):
    try:
        return all(any(item.ready for item in row) for row in values)
    except RuntimeError:
        return False


def with_protected_nested(values, manager):
    with manager:
        return any(all(item.ready for item in row) for row in values)


def inner_filter_nested(values):
    return all(any(item.ready for item in row if item.visible) for row in values)


def outer_filter_nested(values):
    return any(all(item.ready for item in row) for row in values if row)


def boolean_inner_element_nested(values):
    return all(any(item.ready and item.valid for item in row) for row in values)


def tuple_input_control(values):
    return all(tuple([item.ready for item in row]) for row in values)


def list_comprehension_input_control(values):
    return any([item.ready for item in row] for row in values)


def nonnested_all_control(values):
    return all(item.ready for item in values)


def nonnested_any_control(values):
    return any(item.ready for item in values)


def keyword_call_control(values):
    return all(all(iterable=(item.ready for item in row)) for row in values)


def starred_call_control(values):
    return any(all(*(item.ready for item in row)) for row in values)


def method_call_control(values, checker):
    return all(checker.all(item.ready for item in row) for row in values)


def sum_outer_control(values):
    return sum(any(1 if item.ready else 0 for item in row) for row, _ in values)


def generator_filter_call_control(values):
    return all(row for row in values if any([item.ready for item in row]))


def nested_negated_inner_any_boolean_control(values, enabled):
    return all(enabled and not any(item.ready for item in row) for row in values)


def nested_negated_inner_all_if_control(values, enabled):
    return any((not all(item.ready for item in row)) if enabled else row for row in values)


async def negated_inner_any_coroutine_assert_control(values):
    assert all(not any(item.ready('x') for item in row) for _, row in values)


async def negated_inner_all_coroutine_assert_control(values):
    assert any(not all(item.ready('x') for item in row) for _, row in values)


async def double_negated_inner_any_coroutine_assert_control(values):
    assert all(not not any(item.ready('x') for item in row) for _, row in values)


async def double_negated_inner_all_coroutine_assert_control(values):
    assert any(not not all(item.ready('x') for item in row) for _, row in values)


async def unpacked_inner_any_coroutine_assert_control(values):
    assert all(any(item.ready('x') for item in row) for _, row in values)


async def unpacked_inner_all_coroutine_assert_control(values):
    assert any(all(item.ready('x') for item in row) for _, row in values)


def nested_unpacking_dispatch_control(values):
    return any(all(any(item.ready for item in row) for _, row in group) for group in values)


def parenthesized_nested_all_any(values):
    return all((any((item.ready for item in row)) for row in values))


def multiline_commented_nested_all_any(values):
    return all(
        # outer before
        any(
            # inner before
            item.ready for item in row
            # inner after
        )
        for row in values
        # outer after
    )


def nested_iterable_all_any(values):
    return all(any(item for item in all(value for value in row)) for row in values)


def nested_iterable_any_all(values):
    return any(all(item for item in any(value for value in row)) for row in values)


def ordinary_generator_iterable_control(values):
    return all(sum(item for item in any(value for value in row)) for row in values)


def nested_ordinary_generator_iterable_control(values):
    return any(sum(item for item in all(any(part for part in value) for value in row)) for row in values)


async def coroutine_ordinary_generator_iterable_control(values):
    assert all(sum(item for item in any(value for value in row)) for row in values)


async def asynchronous_generator_iterable_control(values):
    return all(sum(item async for item in any(value for value in row)) for row in values)
";
        assert_matches_cpython_marshal(source, "nested_boolean_generator_dispatch.py");
    }

    #[test]
    fn matches_cpython_marshal_for_iterator_boolean_generator_dispatch_owners() {
        let source = r"def direct_any(groups):
    for values in groups:
        if any(not value for value in values):
            continue
        yield values


def direct_all(groups):
    for values in groups:
        if all(value for value in values):
            break
        yield values


def disjunction(groups):
    for values in groups:
        if not values or any(not value for value in values):
            continue
        yield values


def conjunction(groups, first, second):
    for values in groups:
        if first and second and all(value for value in values):
            continue
        yield values


def prior_yield(groups):
    for values in groups:
        yield None
        if not values or any(not value for value in values):
            continue
        yield values


def prior_assignment(groups):
    for group in groups:
        values = [value for value in group]
        if not values or any(not value for value in values):
            return
        yield values


def first_yield_else(groups, flag):
    for values in groups:
        if flag or any(not value for value in values):
            yield values
        else:
            consume(values)


def first_yield_elif(groups, flag, other):
    for values in groups:
        if flag or all(value for value in values):
            yield values
        elif other:
            consume(values)


def boolean_value(groups, flag, sink):
    for values in groups:
        result = flag or sink((any(not value for value in values), values))
        yield result


def plain(groups, flag):
    for values in groups:
        if flag or any(not value for value in values):
            continue
        consume(values)


async def coroutine(groups, first, second):
    for values in groups:
        if first and second and any(not value for value in values):
            continue
        await consume(values)


async def coroutine_after_await(groups, flag):
    for values in groups:
        await prepare(values)
        if flag or all(value for value in values):
            continue
        await consume(values)


async def coroutine_first_await(groups, flag):
    for values in groups:
        if flag or any(not value for value in values):
            await consume(values)
        else:
            prepare(values)


async def asynchronous_generator(groups, flag):
    for values in groups:
        if flag or all(value for value in values):
            continue
        yield values
";
        assert_matches_cpython_marshal(source, "iterator_boolean_generator_dispatch.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_boolean_generators_in_comprehensions() {
        let source = r"def porcupine_shaped(items, filepath):
    return consume(
        {
            name: filetype
            for name, filetype in items
            if any(
                fnmatch.fnmatch(filepath.as_posix(), '*/' + pattern)
                for pattern in filetype['filename_patterns']
            )
        },
        str(filepath),
    )


def all_porcupine_shaped(items, filepath):
    return consume(
        {
            name: filetype
            for name, filetype in items
            if all(
                fnmatch.fnmatch(filepath.as_posix(), '*/' + pattern)
                for pattern in filetype['filename_patterns']
            )
        },
        str(filepath),
    )


def outside_comprehension(values):
    return any(value for value in values)
";
        assert_matches_cpython_marshal(source, "optimized_boolean_comprehensions.py");
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_boolean_dispatch_guard_owners() {
        let cases = [
            (
                "optimized_boolean_guard_store_fusion.py",
                r"def protected_by_target_store_fusion(items):
    return [
        (left, right)
        for left, right in items
        if any(value for value in right)
    ]
",
            ),
            (
                "optimized_boolean_guard_prior_call.py",
                r"def protected_by_prior_call(groups):
    return [
        (probe(group), any(value for value in group))
        for group in groups
    ]
",
            ),
            (
                "optimized_boolean_guard_filter.py",
                r"def simple_filter(groups):
    return [group for group in groups if any(value for value in group)]
",
            ),
            (
                "optimized_boolean_guard_captured_target.py",
                r"def captured_target(items, predicate):
    return [
        (left, right)
        for left, right in items
        if all(predicate(right, value) for value in right)
    ]
",
            ),
            (
                "optimized_boolean_guard_element.py",
                r"def comprehension_element(groups):
    return [any(value for value in group) for group in groups]
",
            ),
            (
                "optimized_boolean_guard_later_iterable.py",
                r"def later_iterable(groups):
    return [
        value
        for group in groups
        for value in any(item for item in group)
    ]
",
            ),
            (
                "optimized_boolean_guard_after_condition.py",
                r"def after_condition(groups):
    return [
        group
        for group in groups
        if probe(group)
        if any(value for value in group)
    ]
",
            ),
        ];

        for (filename, source) in cases {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_optimized_tuple_calls_in_comprehension_and_branch_exits() {
        let source = r"def comprehension_result(prefix, sizes, dimensions):
    return [prefix + tuple(sizes[dimension] for dimension in core_dimensions)
            for core_dimensions in dimensions]


def branch_result(results, shapes, dtypes):
    if results is None:
        arrays = tuple(make(shape, dtype)
                       for shape, dtype in zip(shapes, dtypes))
    else:
        arrays = tuple(make_like(result, shape, dtype)
                       for result, shape, dtype
                       in zip(results, shapes, dtypes))
    return arrays
";
        assert_matches_cpython_marshal(source, "optimized_tuple_branch_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_branch_result_ownership() {
        let source = "def ternary(name, suffix):\n    return name[:-len(suffix)] if name.endswith(suffix) else name\n\ndef simple(condition, left, right):\n    return left if condition else right\n\ndef terminal_attribute(condition, holder, left, right):\n    holder.value = left.value if condition else right\n\ndef boolean(left, right):\n    return left or right\n\ndef nested(condition, name):\n    return name.upper() if condition else name\n\ndef nested_boolean(left, middle, right):\n    return (left and middle) or right, (left or middle) and right\n\ndef folded_annotation(arg: 2 or None | int):\n    pass\n\nlambda_conditional = lambda condition, left, right: left if condition else right\nlambda_boolean = lambda left, right: left or right\nmodule_boolean = (left and middle) or right\n";
        assert_matches_cpython_marshal(source, "ownership.py");
    }

    #[test]
    fn matches_cpython_marshal_for_recursive_tail_conditional_lambdas() {
        let source = r"lambda_else = lambda outer, inner, first, second, third: (
    first if outer else second if inner else third
)
lambda_body = lambda outer, inner, first, second, third: (
    (first if inner else second) if outer else third
)


def assignment_control(holder, outer, inner, first, second, third):
    holder.value = (
        (first if inner else second) if outer else third
    )
";
        assert_matches_cpython_marshal(source, "recursive_tail_conditional_lambdas.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_folded_dictionary_comprehension_key() {
        let source = "def folded_key():\n    return {x if True else y: y for x in range(10) for y in range(10)}\n";
        assert_matches_cpython_marshal(source, "folded_key.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_named_comprehension_target() {
        let source = "[value for value in values]\n{key: (left, right) for key, (left, right) in values}\nmodule_result = {last := value for value in range(2)}\nnested_filter = [(key, value) for key, value in pairs if key in [field.name for field in fields]]\nnested_iterable = [(left, right) for left in values for right in [item.value for item in items]]\ngenerator_assignment = list((captured := value for value in values))\n\ndef f():\n    exponential, base_multiplier = 1, 2\n    hash_map = {\n        (exponential := (exponential * base_multiplier) % 3): i + 1\n        for i in range(2)\n    }\n    return hash_map\n";
        assert_matches_cpython_marshal(source, "named_comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_first_iterable_comprehension_target_order() {
        let source = r"def first_iterable_targets(values):
    return [outer for outer in [inner for inner in values]]


def body_targets(values):
    return [[inner for inner in values] for outer in values]


def later_iterable_targets(values):
    return [(outer, inner) for outer in values for inner in [nested for nested in values]]


def generator_first_iterable(values):
    return [outer for outer in (inner for inner in values)]
";
        assert_matches_cpython_marshal(source, "first_iterable_comprehension_targets.py");
    }

    #[test]
    fn matches_cpython_marshal_for_flat_function_local_order() {
        let source = r"@decorate(header_value)
def ordering(values, *, fallback=default_value):
    import package.part as imported
    from elsewhere import value as alias
    forward = later
    container[index] = forward
    for item in values:
        with manager(item) as resource:
            result = resource
    later = result
    return imported, alias, forward, item, later, fallback
";
        assert_matches_cpython_marshal(source, "flat_function_local_order.py");
    }

    #[test]
    fn matches_cpython_marshal_for_leaf_expression_and_scope_fast_paths() {
        let source = r#"class Container:
    def method(self, __value):
        self.__field = __value
        result = self.__field
        del self.__field
        return result, super().__repr__()


def leaves(values, source):
    first_small = 7
    later_small = 7
    huge = 123456789012345678901234567890
    negative_zero = -0.0
    complex_value = 2j
    text = "implicit " "concatenation"
    data = b"adjacent " b"bytes"
    observed = ((captured := item) for item in values)
    wrapped = [item for item in values].__class__
    named = (captured := source).__class__
    return (
        __debug__, None, True, False, ..., first_small, later_small, huge,
        negative_zero, complex_value, text, data, observed, wrapped, named,
    )


def yielding(values):
    return (yield values).__class__


async def nested(values):
    return [(__debug__, item, b"x", "y") async for item in values]
"#;
        assert_matches_cpython_marshal(source, "leaf_expression_and_scope_fast_paths.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_expression_local_order() {
        let source = r"def chained():
    return (outer := (inner := value))

lambda_body = lambda: (lambda_outer := (lambda_inner := lambda_value))

def mixed_global():
    global global_outer
    result = (global_outer := (local_inner := local_value))
    local_value = 1
    return result

def list_element_late_load(items):
    result = [(outer := (inner := seed)) for item in items]
    seed = 1
    return result

def list_two_filters(items):
    result = [item for item in items if (first := left) if (second := right)]
    left = right = 1
    return result

def set_element(items):
    result = {(outer := (inner := seed)) for item in items}
    seed = 1
    return result

def dict_key_then_value(items):
    result = {
        (key_target := key_seed): (value_target := value_seed)
        for item in items
    }
    key_seed = value_seed = 1
    return result

def nested_list(rows):
    result = [[(outer := (inner := seed)) for item in row] for row in rows]
    seed = 1
    return result

def generator_reverse_names(items):
    return ((a_outer := (z_inner := item)) for item in items)

def decorated():
    @(decorator_outer := (decorator_inner := decorator_value))
    def nested():
        pass

    decorator_value = 1
    return nested
";
        assert_matches_cpython_marshal(source, "named_expression_local_order.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_else_local_order() {
        let source = r"def ordered():
    try:
        initial = produce()
    except Exception as error:
        handler_only = fallback()
        result = handler_only
    else:
        result = success(initial)
    return result


def no_else_control():
    try:
        initial = produce()
    except Exception as error:
        handler_only = fallback()
    return initial
";
        assert_matches_cpython_marshal(source, "try_else_local_order.py");
    }

    #[test]
    fn matches_cpython_marshal_for_annotation_only_local_order() {
        let source = r"def tuple_target(values):
    annotated: object
    first, annotated = values
    return first, annotated


def branch_target(condition):
    later: object
    earlier: object
    if condition:
        middle = 1
        earlier = 2
    else:
        later = 3
    return middle, earlier, later


def comprehension_target(values):
    result: list
    result = [item for item in values]
    return result


def annotated_comprehension_target(values):
    item: object
    first = 1
    result = [item for item in values]
    return first, result


def definition_target():
    callback: object
    prior = 1

    def callback():
        pass

    return prior, callback


def captured_only():
    captured: object
    first = 1

    def inner():
        return captured

    return first, inner


def read_target():
    annotated: object
    return annotated


def self_referencing_target():
    target: object
    rhs: object
    first = 1
    target = rhs + target


def augmented_target():
    target: object
    rhs: object
    first = 1
    target += rhs


def stored_subscript_target():
    container: object
    index: object
    rhs: object
    first = 1
    container[index] = rhs


def augmented_subscript_target():
    container: object
    index: object
    rhs: object
    first = 1
    container[index] += rhs


def annotated_attribute_target():
    container: object
    first = 1
    container.value: object


def annotated_subscript_target():
    container: object
    index: object
    first = 1
    container[index]: object


def deleted_subscript_target():
    container: object
    index: object
    first = 1
    del container[index]


def loop_attribute_target(values):
    container: object
    first = 1
    for container.value in values:
        pass


def context_attribute_target(context):
    container: object
    first = 1
    with context as container.value:
        pass


def comprehension_attribute_target(values):
    container: object
    first = 1
    result = [container.value for container.value in values]
    return first, result


def complex_comprehension_order():
    source: object
    container: object
    first = 1
    result = [0 for container.value in source]
    return first, result


def mixed_comprehension_order(values):
    container: object
    index: object
    named: object
    first = 1
    result = [named for container[index], named in values]
    return first, result


def comprehension_source_order():
    source: object
    item: object
    first = 1
    result = [item for item in source]
    return first, result


def comprehension_outer_iterable_order():
    earlier: object
    target: object
    first = 1
    target = [item for item in (earlier, target)]
    return target


def nested_comprehension_order(outer):
    result = [[nested for nested in inner] for item in outer]
    inner = ()
    return result


def exception_target():
    exception: object
    exception_type: object
    first = 1
    try:
        pass
    except exception_type as exception:
        pass


def lambda_default():
    defaulted: object
    first = 1
    callback = lambda value=defaulted: value
    return callback


def generator_iterable():
    iterable: object
    first = 1
    generated = (value for value in iterable)
    return generated
";
        assert_matches_cpython_marshal(source, "annotation_only_local_order.py");
    }

    #[test]
    fn matches_cpython_marshal_for_compound_none_comparison_try_owners() {
        let source = r"def direct_control(value):
    try:
        if value is not None:
            return 1
    except Exception:
        pass
    return 0


def compound(first, second, third, fourth, fifth):
    try:
        if ((first is not None and second is not None)
                or (third is not None and fourth is not None)
                or fifth is not None):
            return 1
    except Exception:
        pass
    return 0


def leading_comparisons(a, b, first, second):
    try:
        if a >= 0 and b >= 0 and (first is not None or second is not None):
            return 1
    except Exception:
        pass
    return 0
";
        assert_matches_cpython_marshal(source, "compound_none_comparison_try_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_while_try_else_borrow_entry() {
        for condition in [
            "flag",
            "not owner.ready",
            "owner.ready is None",
            "owner.a and owner.b",
        ] {
            let source = format!(
                "def f(owner, items, flag):\n    for item in items:\n        try:\n            while True:\n                if flag:\n                    break\n                owner.work()\n        except Exception as error:\n            if flag:\n                break\n            owner.report(error)\n        else:\n            if {condition}:\n                owner.work()\n            owner.report(item)\n"
            );
            assert_matches_cpython_marshal(&source, "terminal_while_try_else_borrow_entry.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_nested_loop_finally_borrow_entry() {
        for inner in [
            "if flag:\n    owner.work()\nelse:\n    break\n",
            "try:\n    owner.work()\nexcept Error:\n    continue\nelse:\n    if flag:\n        owner.work()\n    else:\n        break\n",
        ] {
            for tail in ["return flag", "owner.work()"] {
                let mut indented = String::new();
                for line in inner.lines() {
                    writeln!(indented, "                {line}").unwrap();
                }
                let source = format!(
                    "def f(owner, flag):\n    try:\n        while True:\n            if flag:\n                break\n            while True:\n{indented}        {tail}\n    finally:\n        if owner.ready:\n            owner.finish()\n"
                );
                assert_matches_cpython_marshal(&source, "nested_loop_finally_borrow_entry.py");
            }
        }
    }

    #[test]
    fn matches_cpython_marshal_for_retained_try_else_borrow_fallthrough() {
        let source = r"def terminal_else_if(value, flag):
    try:
        action()
    except Exception:
        pass
    else:
        if flag:
            return flag
    consume(value)


def fallthrough_else_if_control(value, flag):
    try:
        action()
    except Exception:
        pass
    else:
        if flag:
            consume(flag)
    consume(value)


def terminal_and_fallthrough_arms(value, flag):
    try:
        action()
    except Exception:
        pass
    else:
        if flag:
            return flag
        else:
            other()
    consume(value)


def two_terminal_arms_control(value, flag):
    try:
        action()
    except Exception:
        pass
    else:
        if flag:
            return flag
        else:
            raise RuntimeError
    consume(value)
";
        assert_matches_cpython_marshal(source, "retained_try_else_borrow_fallthrough.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_conditional_expressions() {
        let source = "value = (\n    'body'\n    if True\n    else 'otherwise'\n)\n\ndef nested():\n    target.attribute = (\n        (first if inner else second)\n        if outer\n        else third\n    )\n";
        assert_matches_cpython_marshal(source, "conditional_expression.py");
    }

    #[test]
    fn matches_cpython_marshal_for_conditional_expression_join_locations() {
        let source = r"def split_short_circuit(manager, value):
    manager.first = (value.items or []) if has_value(
        value, 'items') else []
    manager.second = (value.other or []) if has_value(
        value, 'other') else []


def nested_body(outer, inner, first, second, third):
    value = (first if inner else second) if outer else third
    consume(value)


def recursive_nested_body(outer, middle, inner, first, second, third, fourth):
    value = ((first if inner else second) if middle else third) if outer else fourth
    consume(value)


def folded_true_nested_body(outer, first, second, third):
    value = (first if True else second) if outer else third
    consume(value)


def folded_false_nested_body(outer, first, second, third):
    value = (first if False else second) if outer else third
    consume(value)


def nested_short_circuit_fallthrough(outer, inner, first, second, third, fourth):
    value = (first if inner else (second or third)) if outer else fourth
    consume(value)


def nested_else_control(outer, inner, first, second, third):
    value = first if outer else (second if inner else third)
    consume(value)


def return_control(outer, inner, first, second, third):
    return (first if inner else second) if outer else third


def same_block_control(condition, body, otherwise):
    return body if condition else otherwise


def call_control(condition):
    return body() if condition else otherwise()
";
        assert_matches_cpython_marshal(source, "conditional_expression_join_locations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_conditional_call_returns_in_try_bodies() {
        let source = r#"def direct(flag):
    try:
        return build(value="a" if flag else "b")
    except Error:
        return failure()


def named_handler(flag):
    try:
        return build(value="a" if flag else "b")
    except Error as error:
        return failure(error)


def preceding_call(flag):
    try:
        prepare()
        return build(value="a" if flag else "b")
    except Error:
        return failure()


def positional(flag):
    try:
        return build("a" if flag else "b")
    except Error:
        return failure()


def trailing_argument_control(flag, value):
    try:
        return build(first=value, selected="a" if flag else "b", final=value)
    except Error:
        return failure()


def outside_try_control(flag):
    return build(value="a" if flag else "b")


async def awaited_control(client, flag):
    try:
        value = await client.load()
        return build(value="a" if flag else "b", loaded=value)
    except Error as error:
        return failure(error)


def protected_with_control(manager, flag):
    try:
        with manager:
            return build(value="a" if flag else "b")
    except Error:
        return failure()


def formatted_return(flag):
    try:
        return f"Selected: {'yes' if flag else 'no'}"
    except Error:
        return failure()


def same_line_components(flag, value):
    try:
        if value:
            return f"Value: {value}\\n" f"Attribute: {value.attribute}\\n" f"Selected: {'yes' if flag else 'no'}"
        return "missing"
    except Error:
        return failure()


def multiline_call_arguments(flag):
    try:
        return build(
            value="a" if flag else "b",
        )
    except Error:
        return failure()


def parenthesized_call_control(flag):
    try:
        return (
            build(value="a" if flag else "b")
        )
    except Error:
        return failure()


def multi_component_format_control(flag, value):
    try:
        if value:
            return (
                f"Value: {value}\\n"
                f"Attribute: {value.attribute}\\n"
                f"Selected: {'yes' if flag else 'no'}"
            )
        return "missing"
    except Error:
        return failure()
"#;
        assert_matches_cpython_marshal(source, "conditional_call_returns_in_try_bodies.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unpacked_class_arguments() {
        let source = "class C1(Generic[T], str, **{'metaclass': type}):\n    ...\n\nclass C2(Generic[T], str, metaclass=type):\n    ...\n\nclass C3(Generic[T], metaclass=type, *[str]):\n    ...\n\nclass C4(x=1, **y):\n    ...\n";
        assert_matches_cpython_marshal(source, "class_arguments.py");
    }

    #[test]
    fn matches_cpython_marshal_for_import_originated_calls() {
        let source = "import sys\n\ndef reimported():\n    import sys\n    sys.exit(1)\n\ndef shadowed(sys):\n    sys.exit(1)\n\ndef only_local():\n    import os\n    os._exit(1)\n";
        assert_matches_cpython_marshal(source, "imported_calls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_future_import_name_call_collisions() {
        let source = r"from __future__ import annotations, generator_stop as stopped

def feature_name(annotations):
    return annotations.items()

def aliased_feature_name(stopped):
    return stopped.items()

def unrelated_name(receiver):
    return receiver.items()
";
        assert_matches_cpython_marshal(source, "future_import_name_calls.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_nested_if_branch_exit() {
        let source = "seed = 0\nif True:\n    body()\ndef between():\n    pass\nreused = True\n\nif a:\n    if b:\n        body()\nelif c:\n    other()\nafter1(); after2(); after3(); after4(); after5(); after6()\n\nif x:\n    if True:\n        body()\nelif y:\n    other()\n\nif False if True else False:\n    body()\nelif True:\n    other()\nelse:\n    unreachable()\n\ndef conditional_in_loop(src, dst):\n    for k, v in src:\n        if True if True else False:\n            dst[k] = v\n\ndef terminating_nested_if(x, y):\n    if x:\n        if y:\n            raise Exception()\n        else:\n            body()\n    else:\n        other()\n    after()\n\ndef nested_if_before_implicit_return(x, y):\n    if x:\n        if y:\n            return\n        else:\n            value = 2\n    else:\n        value = 3\n    return\n\ndef nested_if_with_fallthrough_return(x, y):\n    if x:\n        if y:\n            return y\n    else:\n        return x\n    return None\n\ndef nested_if_before_loop_control(items, condition):\n    for item in items:\n        if item:\n            if condition:\n                value = 1\n            else:\n                value = 2\n            continue\n        else:\n            value = 3\n        continue\n\ndef protected_loop_control(items):\n    for item in items:\n        try:\n            body()\n            if item:\n                break\n        except Exception:\n            recover()\n    for item in items:\n        try:\n            body()\n            continue\n        except Exception:\n            recover()\n\ndef nested_break_then_return(items, condition):\n    for item in items:\n        if item:\n            if condition:\n                break\n        else:\n            return condition\n        return None\n\nif True:\n    if first:\n        one()\n    elif second:\n        two()\n";
        assert_matches_cpython_marshal(source, "nested_if.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_assertion_branch_exit() {
        let source = r#"def select(result, states, failed):
    if failed:
        assert states == {"disabled"}
    else:
        assert states == {"normal"}

    return result
"#;
        assert_matches_cpython_marshal(source, "assert_branch_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_terminal_try_for_break_exit() {
        let source = r"def target():
    try:
        response = 42
        for value in range(100):
            if value == response:
                break
    except ValueError as error:
        if not error.args:
            raise
";
        assert_matches_cpython_marshal(source, "terminal_try_for_break_exit.py");
    }

    #[test]
    fn matches_cpython_marshal_for_later_assertion_branches_before_function_exits() {
        let source = r"def before_return(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    return


def before_raise(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    raise RuntimeError


def ordinary_successor_control(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    after()


def terminating_else_control(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        return
    after()


def terminating_else_before_return(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        return
    return


def loop_controls(items, first, second):
    for item in items:
        if first:
            consume()
        elif second:
            assert second.value
        else:
            consume()
        break
    for item in items:
        if first:
            consume()
        elif second:
            assert second.value
        else:
            consume()
        continue
";
        assert_matches_cpython_marshal(source, "later_assertion_function_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_constant_false_assertions_in_final_else_suites() {
        let source = r#"def direct(condition, value):
    if condition:
        return
    else:
        assert False, value


def chained(kind, value):
    if kind == "first":
        first()
    elif kind == "second":
        second()
    else:
        assert 0, f"unknown: {value}"


def control(value):
    assert False, value
"#;
        assert_matches_cpython_marshal(source, "constant_false_assertion_else.py");
    }

    #[test]
    fn matches_cpython_marshal_for_assertion_chained_comparison_exit_nops() {
        let source = r#"def direct_chain(first, middle, last):
    assert first.value < middle.value < last.value


def long_chain(first, second, third, fourth):
    assert first.value == second.value == third.value == fourth.value


def chain_with_message(first, middle, last):
    assert first.value <= middle.value <= last.value, "out of order"


def simple_comparison(first, last):
    assert first.value < last.value


def non_assertion_chain(first, middle, last):
    if first.value < middle.value < last.value:
        consume()
    after()
"#;
        assert_matches_cpython_marshal(source, "assert_chained_comparison_exit_nops.py");
    }

    #[test]
    fn matches_cpython_marshal_for_simple_comparison_condition_locations() {
        let source = r"def assertions(first, second):
    assert first.value < second.value
    assert (
        first.value
        <= second.value
    )
    assert first.value != second.value


def branches(first, second, predicate):
    if first.value == second.value:
        consume(first)
    if (
        first.value
        >= second.value
    ):
        consume(second)
    if predicate(first, second):
        consume(first, second)
";
        assert_matches_cpython_marshal(source, "simple_comparison_condition_locations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_assertion_conditional_expression_exit_nops() {
        let source = r#"def direct(condition, first, second):
    assert (
        first == 1 if condition else second == 2
    )


def split(condition, first, second):
    assert (
        first == 1
        if condition
        else second == 2
    )
    consume(first)


def message(condition, first, second):
    assert (
        first == 1
        if condition
        else second == 2
    ), "message"


async def asynchronous(condition, first, second):
    assert (
        first == 1 if condition else second == 2
    )


def try_finally(condition, first, second):
    try:
        assert (
            first == 1 if condition else second == 2
        )
    finally:
        cleanup()


def same_line_control(condition, first, second):
    assert first == 1 if condition else second == 2


def same_start_multiline_control(condition, first, second):
    assert (first == 1
            if condition
            else second == 2)


def ordinary_assert_control(first, second):
    assert (
        first == second
    )


def ordinary_if_control(condition, first, second):
    if (
        first == 1 if condition else second == 2
    ):
        consume(first)
"#;
        assert_matches_cpython_marshal(source, "assert_multiline_ifexp_exit_nops.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_tail_assertion_conditional_expression_copies() {
        let source = r"def sync_split(items, expected, first, second):
    for item in items:
        assert (
            first(item)
            if item == expected
            else second(item)
        )


def sync_same_line(items, condition, first, second):
    for item in items:
        assert first(item) if condition else second(item)


async def coroutine_split(wait, items, expected, first, second):
    await wait()
    for item in items:
        assert (
            first(item)
            if item == expected
            else second(item)
        )


def generator_split(items, expected, first, second):
    yield first
    for item in items:
        assert (
            first(item)
            if item == expected
            else second(item)
        )


def call_selector_non_tail_control(items, condition, first, second):
    for item in items:
        assert first(item) if condition(item) else second(item)
        consume(item)


def non_final_position_control(items, selected, expected, first, second):
    for item in items:
        if selected(item):
            assert first(item) if item == expected else second(item)
            consume(item)
        else:
            reject(item)
";
        assert_matches_cpython_marshal(source, "assert_loop_tail_ifexp_copies.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_negated_chained_comparison_exit_nops() {
        let source = r#"def direct_raise(low, value, high):
    if not (low <= value < high):
        raise IndexError
    consume()


def prefixed_elif(kind, enabled, low, value, high):
    if kind == "simple":
        consume()
    elif enabled and not (low <= value < high):
        raise IndexError
    finish()


def direct_return(low, value, high):
    if not (low <= value < high):
        return value
    consume()


def two_negated_chains(a, b, c, d, e, f):
    if not (a < b < c) and not (d < e < f):
        raise ValueError
    finish()


def terminating_else(low, value, high):
    if not (low <= value < high):
        raise IndexError
    else:
        raise ValueError


def loop_raise_after_try(items, enabled, low, value, high):
    for item in items:
        try:
            prepare(item)
        except ValueError:
            recover()
        if item is None:
            consume(item)
        elif enabled and not (low <= value < high):
            raise IndexError
        finish(item)


def final_operand_control(low, value, high, enabled):
    if not (low <= value < high) and enabled:
        raise IndexError
    consume()


def multi_statement_control(low, value, high):
    if not (low <= value < high):
        consume()
        raise IndexError
    finish()


def loop_controls(items, low, value, high):
    for item in items:
        if not (low <= value < high):
            break
        consume(item)
    for item in items:
        if not (low <= value < high):
            continue
        consume(item)


def loop_return(items, low, high):
    for item in items:
        if not (low <= item <= high):
            return False
    return True
"#;
        assert_matches_cpython_marshal(source, "terminal_negated_chained_comparison_exit_nops.py");
    }

    #[test]
    fn matches_cpython_marshal_for_positive_chained_comparison_small_exits() {
        let source = r"def tuple_return(left, right, limit):
    if left == right == limit:
        return left, left
    consume()
    return fallback()


def direct_raise(left, right, limit):
    if left == right == limit:
        raise ValueError
    consume()
    return fallback()


def multi_statement_control(left, right, limit):
    if left == right == limit:
        consume(left)
        return left, left
    consume()
    return fallback()


def final_none_control(first, second):
    if first is second is None:
        return (None,) * 4
    consume()
    return fallback()
";
        assert_matches_cpython_marshal(source, "positive_chained_comparison_small_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_none_chained_comparison_jumps() {
        let source = r#"class Example:
    def __init__(self, first, second=None, prefix=None):
        if second is prefix is None:
            self.value = first
            self.first = None
            self.second = None
        else:
            self.value = prefix
            self.first = first
            self.second = second

    def render(self):
        first = self.first
        second = self.second
        if first is second is None:
            return self.value
        else:
            return f"{first}: {second}"
"#;
        assert_matches_cpython_marshal(source, "final_none_chained_comparison.py");
    }

    #[test]
    fn matches_cpython_marshal_for_final_try_assertion_pop_blocks() {
        let source = r#"def direct(value):
    try:
        assert value
    except ValueError:
        recover()
    return value


def message(value):
    try:
        assert value, "bad"
    except ValueError:
        recover()
    return value


def with_else(value):
    try:
        assert value
    except ValueError:
        recover()
    else:
        consume(value)
    return value


def multiple_handlers(value):
    try:
        assert value
    except ValueError:
        recover()
    except TypeError:
        recover()
    return value


def nonfinal(value):
    try:
        assert value
        consume(value)
    except ValueError:
        recover()
    return value


def constant_true(value):
    try:
        assert True
    except ValueError:
        recover()
    return value


def constant_false(value):
    try:
        assert False
    except AssertionError:
        recover()
    return value


def boolean_and(left, right):
    try:
        assert left and right
    except ValueError:
        recover()
    return right


def boolean_or(left, right):
    try:
        assert left or right
    except ValueError:
        recover()
    return right


def conditional(value, alternate, flag):
    try:
        assert value if flag else alternate
    except ValueError:
        recover()
    return value


def protected(manager, value):
    with manager:
        try:
            assert value
        except ValueError:
            recover()
    return value


def protected_loop(manager, values):
    with manager:
        for value in values:
            try:
                assert value
            except ValueError:
                recover()


def terminal(value):
    try:
        assert value
    except ValueError:
        recover()


def generator(value):
    try:
        assert value
    except ValueError:
        recover()
    yield value


async def coroutine(value):
    try:
        assert value
    except ValueError:
        recover()
    return value
"#;
        assert_matches_cpython_marshal(source, "final_try_assertions.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_if_final_clause_exits() {
        let source = r"def final_else(outer, inner):
    if outer:
        if inner:
            first()
        else:
            second()
    else:
        other()
    after()

def terminal_final_elif(outer, inner, last):
    if outer:
        if inner:
            first()
        elif last:
            raise Error()
    else:
        other()
    after()

def nonterminal_final_elif(outer, inner, last):
    if outer:
        if inner:
            raise Error()
        elif last:
            body()
    else:
        other()
    after()

def terminal_final_else(outer, inner):
    if outer:
        if inner:
            first()
        else:
            raise Error()
    else:
        other()
    after()

def no_else_nonterminal(outer, inner):
    if outer:
        if inner:
            body()
    else:
        other()
    after()

def folded_nested(outer):
    if outer:
        if True:
            body()
    else:
        other()
    after()

def threaded_three_way_cascade(major, minor, bugfix, other_major, other_minor, other_bugfix):
    if major == other_major:
        if minor == other_minor:
            if bugfix == other_bugfix:
                result = 0
            elif bugfix > other_bugfix:
                result = 1
            else:
                result = -1
        elif minor > other_minor:
            result = 1
        else:
            result = -1
    elif major > other_major:
        result = 1
    else:
        result = -1
    return result

def cascade_with_leading_statement(outer, middle, inner):
    if outer:
        prepare()
        if middle:
            if inner:
                first()
            elif alternate():
                second()
            else:
                third()
        elif alternate():
            fourth()
        else:
            fifth()
    elif alternate():
        sixth()
    else:
        seventh()
    after()
";
        assert_matches_cpython_marshal(source, "nested_if_final_clause.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_unreachable_assertion_message() {
        let source = "seed = '\u{a0}'\nassert True, print('hidden', print.__name__, sep=None)\ncondition = 'used'\n\ndef loop_tail_assert(values):\n    for value in values:\n        assert value\n";
        assert_matches_cpython_marshal(source, "assert.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_formatted_not_equal_t_string() {
        let source = "t\"{3!=4:}\"\n";
        assert_matches_cpython_marshal(source, "t_string.py");
    }

    #[test]
    fn matches_cpython_marshal_for_interpolated_string_boundaries() {
        let source = r#"
def render(item, nested):
    rf'raw\path {item}'
    f'''first
{item!r}
# literal
last'''
    r'left' f"middle {item}" '''right'''
    t'left' tr"""right {item}
# literal"""
    f"""start {(
        {'}': nested['{']}  # expression comment
    ) = } end"""
    t"""{(
        nested["key"]  # expression comment
    ) = }"""
"#;
        assert_matches_cpython_marshal(source, "interpolated_string_boundaries.py");
    }

    #[test]
    fn matches_cpython_marshal_for_debug_interpolation_text() {
        let source = r#"
def render(item, mapping, width):
    f"{  item   =   !r}"
    t"{  item   =   !r}"
    t"""{(
        mapping["="]  # = inside comment
    )   =   :>{width}}"""
    t"""{f"{item=} = literal" = !s:^10}"""
    f"""{item=  # after marker
}"""
    t"""{item=  # after marker
}"""
"#;
        assert_matches_cpython_marshal(source, "debug_interpolation_text.py");
    }

    #[test]
    fn matches_cpython_marshal_for_comment_trivia_in_interpolations() {
        let source = r##"
def render(value):
    f"""{(
        '# single',
        "# double",
        r'# raw',
        '''# triple''',
        """# triple double""",
    ) =  # debug comment
    }"""
    t"""{"escaped \\\" # still string" = }"""
    t"""{f"nested # {value}" = !r:^20}"""
    t"""{(
        '# literal',
        f"nested # {value}",
    )  # expression comment
    !r:^20}"""
"##;
        assert_matches_cpython_marshal(source, "comment_trivia_interpolations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_irrefutable_constant_match_guard() {
        let source = "match subject:\n    case value if True:\n        pass\n    case _:\n        unreachable()\nafter()\n";
        assert_matches_cpython_marshal(source, "match.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sequence_wildcard_star() {
        let source = "def prefix(value):\n    match value:\n        case [captured, *_]:\n            return captured\n\ndef suffix(value):\n    match value:\n        case [*_, captured]:\n            return captured\n\ndef both(value):\n    match value:\n        case [first, *_, last]:\n            return first, last\n\ndef nested(value):\n    match value:\n        case [[captured], *_]:\n            return captured\n";
        assert_matches_cpython_marshal(source, "sequence_wildcard.py");
    }

    #[test]
    fn matches_cpython_marshal_for_yield_in_try_finally() {
        let source = "def generator(flag):\n    if flag:\n        try:\n            yield\n        finally:\n            pass\n    else:\n        yield\n\ndef terminating_try_except_finally():\n    try:\n        raise StopIteration\n    except ValueError:\n        yield 1\n    finally:\n        pass\n";
        assert_matches_cpython_marshal(source, "generator.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_generator_qualnames() {
        let source = "def nested(groups):\n    return (sum(item for item in group) for group in groups)\n\ndef lambda_inside(values):\n    return ((lambda: value) for value in values)\n\nresult = consume(\n    # leading\n    item for item in values\n    # trailing\n)\n";
        assert_matches_cpython_marshal(source, "nested_generators.py");
    }

    #[test]
    fn matches_cpython_marshal_for_local_borrowing_across_exception_regions() {
        let source = "def try_return():\n    try:\n        value = process()\n        return value\n    except ValueError:\n        pass\n\ndef with_return(manager):\n    with manager:\n        return 1\n\ndef match_subject(provided):\n    match provided:\n        case True:\n            return captured\n\ndef comprehension(collection):\n    return [element for element in collection if element is not None]\n\ndef repeated_checked_load():\n    value = value[value.attribute == 'key']\n\ndef maybe_unbound(condition):\n    if condition:\n        value = 1\n    else:\n        use()\n    return value\n\ndef consecutive_try_imports():\n    try:\n        from first import First\n    except ImportError:\n        from fallback import First\n    First()\n    try:\n        from second import Second\n    except ImportError:\n        from fallback import Second\n    Second()\n\ndef folded_false(value):\n    if value == 1:\n        return 1\n    elif False:\n        return 2\n    elif value == 3:\n        return 3\n\ndef loaded_before_binding():\n    for item in values:\n        use(values, item)\n    values = []\n\ndef conditional_import(flag):\n    values = []\n    if flag:\n        values.append(flag)\n    from package import imported\n    return values, imported\n";
        assert_matches_cpython_marshal(source, "borrowing.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_with_normal_exits() {
        let source = r"def pass_body(value, manager):
    try:
        with manager:
            pass
    except Error:
        pass
    consume(value)

def active_body(value, manager):
    try:
        with manager:
            action()
    except Error:
        pass
    consume(value)

def following_try(value, manager):
    try:
        with manager:
            action()
    except Error:
        pass
    try:
        consume(value)
    except OtherError:
        raise

def terminal_handler(value, manager):
    try:
        with manager:
            action()
    except Error:
        return
    consume(value)

def nested_try(value, manager):
    try:
        try:
            with manager:
                action()
        except InnerError:
            recover()
        consume(value)
    except OuterError:
        recover_outer()
    return value

def finally_guard(value, manager):
    try:
        with manager:
            action()
    finally:
        cleanup()
    try:
        consume(value)
    except Error:
        raise

def combined_loop(manager, items):
    for item in items:
        try:
            with manager:
                action(item)
        except Error:
            recover()
        finally:
            cleanup()

def combined_following(manager):
    try:
        with manager:
            action()
    except Error:
        recover()
    finally:
        cleanup()
    after()

def combined_terminal(manager):
    try:
        with manager:
            action()
    except Error:
        recover()
    finally:
        cleanup()

def ordinary_guard(value):
    try:
        action()
    except Error:
        pass
    try:
        consume(value)
    except OtherError:
        raise
";
        assert_matches_cpython_marshal(source, "try_with_normal_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_with_suppressed_exit_in_finally_body() {
        let source = r"def terminal_nested(manager):
    try:
        try:
            with manager:
                action()
        except Error:
            recover()
    finally:
        cleanup()


def following_nested(manager):
    try:
        try:
            with manager:
                action()
        except Error:
            recover()
        after()
    finally:
        cleanup()


def with_not_final(manager):
    try:
        try:
            with manager:
                action()
            after()
        except Error:
            recover()
    finally:
        cleanup()


def sole_finally(manager):
    try:
        with manager:
            action()
    finally:
        cleanup()
";
        assert_matches_cpython_marshal(source, "try_with_suppressed_exit_in_finally_body.py");
    }

    #[test]
    fn uses_cpython_owned_loads_after_nested_handler_cold_joins() {
        let source = r#"import dis


def direct_handler_tail(value):
    try:
        result = action()
    except Error:
        try:
            recover()
        except OtherError:
            alternate()
    consume(value)
    return result


def conditional_handler_tail(value, flag):
    try:
        result = action()
    except Error:
        if flag:
            try:
                recover()
            except OtherError:
                alternate()
        else:
            fallback()
    consume(value)
    return result


def loop_handler_tail(value, items):
    try:
        result = action()
    except Error:
        for item in items:
            if item:
                try:
                    recover(item)
                except OtherError:
                    alternate(item)
        result = fallback()
    consume(value)
    return result


def load_after_consume(function):
    instructions = list(dis.get_instructions(function))
    consume = next(
        index
        for index, instruction in enumerate(instructions)
        if instruction.opname == "LOAD_GLOBAL" and instruction.argval == "consume"
    )
    return instructions[consume + 1].opname


for function in (direct_handler_tail, conditional_handler_tail, loop_handler_tail):
    print(load_after_consume(function))
"#;
        assert_executes(source, "LOAD_FAST\nLOAD_FAST\nLOAD_FAST\n");
    }

    #[test]
    fn uses_cpython_borrow_kinds_after_terminal_nested_handlers() {
        let source = r#"import dis


def terminal_nested_handler(value):
    try:
        result = action()
    except Error:
        try:
            recover()
        except OtherError:
            return value
        result = fallback()
    consume(value)
    return result


def fallthrough_nested_handler(value):
    try:
        result = action()
    except Error:
        try:
            recover()
        except OtherError:
            alternate()
        result = fallback()
    consume(value)
    return result


def load_after_consume(function):
    instructions = list(dis.get_instructions(function))
    consume = next(
        index
        for index, instruction in enumerate(instructions)
        if instruction.opname == "LOAD_GLOBAL" and instruction.argval == "consume"
    )
    return instructions[consume + 1].opname


print(load_after_consume(terminal_nested_handler))
print(load_after_consume(fallthrough_nested_handler))
"#;
        assert_executes(source, "LOAD_FAST\nLOAD_FAST_BORROW\n");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_handler_normal_exit_borrowing() {
        let source = r"def bare_pass(value):
    try:
        action()
    except:
        pass
    consume(value)


def bare_assignment(value):
    try:
        result = action()
    except:
        result = None
    consume(value, result)


def branch_local_tail(value, enabled):
    if enabled:
        try:
            action()
        except:
            pass
        consume(value)
    finish()


def loop_local_tail(value, items):
    for item in items:
        try:
            action(item)
        except:
            pass
        consume(value)


def typed_handler_control(value):
    try:
        action()
    except Error:
        pass
    consume(value)


def direct_name_return(value):
    try:
        action()
    except:
        pass
    return value


def shared_branch_join(value, enabled):
    if enabled:
        try:
            action()
        except:
            pass
    consume(value)


def shared_loop_join(value, items):
    for item in items:
        try:
            action(item)
        except:
            pass
    consume(value)
";
        assert_matches_cpython_marshal(source, "bare_handler_normal_exit_borrowing.py");
    }

    #[test]
    fn matches_cpython_marshal_for_empty_try_exit_borrow_boundaries() {
        let source = r"def fallthrough_if(value, flag):
    try:
        if flag:
            consume()
    except Exception:
        pass
    consume(value)
    return value


def terminal_if(value, flag):
    try:
        if flag:
            return value
    except Exception:
        pass
    consume(value)
    return value


def followed_in_try(value, flag):
    try:
        if flag:
            consume()
        other()
    except Exception:
        pass
    consume(value)
    return value


def plain_try(value):
    try:
        consume()
    except Exception:
        pass
    consume(value)
    return value


def shared_outer_join(value, flag, outer):
    if outer:
        try:
            if flag:
                consume()
        except Exception:
            pass
    consume(value)
    return value


def unthreaded_nested_if(value, flag, inner):
    try:
        if flag:
            if inner:
                return value
            else:
                consume()
    except Exception:
        pass
    consume(value)
    return value


def fallthrough_if_in_else(value, flag):
    try:
        consume()
    except Exception:
        pass
    else:
        if flag:
            consume()
    consume(value)
    return value


def complete_if_in_else(value, flag):
    try:
        consume()
    except Exception:
        pass
    else:
        if flag:
            consume()
        else:
            recover()
    consume(value)
    return value
";
        assert_matches_cpython_marshal(source, "empty_try_exit_borrow.py");
    }

    #[test]
    fn uses_cpython_borrow_kinds_for_threaded_empty_try_exits() {
        let source = r#"import dis


def threaded_nested_try(value, flag):
    try:
        if flag:
            try:
                consume()
            except ValueError:
                pass
    except Exception:
        pass
    consume(value)


def threaded_nested_with(value, flag, manager):
    try:
        if flag:
            with manager:
                consume()
    except Exception:
        pass
    consume(value)


def threaded_nested_if(value, flag, inner):
    try:
        if flag:
            if inner:
                consume()
            else:
                other()
    except Exception:
        pass
    consume(value)


def threaded_loop_break(value, flag, inner, items):
    try:
        if flag:
            for item in items:
                if inner:
                    break
                consume(item)
    except Exception:
        pass
    consume(value)


def threaded_match(value, flag, subject):
    try:
        if flag:
            match subject:
                case True:
                    consume()
                case _:
                    other()
    except Exception:
        pass
    consume(value)


def value_load(function):
    loads = [
        instruction.opname
        for instruction in dis.get_instructions(function)
        if instruction.argval == "value"
        and instruction.opname.startswith("LOAD_FAST")
    ]
    assert len(loads) == 1, loads
    return loads[0]


for function in (
    threaded_nested_try,
    threaded_nested_with,
    threaded_nested_if,
    threaded_loop_break,
    threaded_match,
):
    print(value_load(function))
"#;
        assert_executes(
            source,
            "LOAD_FAST_BORROW\nLOAD_FAST_BORROW\nLOAD_FAST_BORROW\nLOAD_FAST_BORROW\nLOAD_FAST_BORROW\n",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_borrowing_after_constant_loop_breaks() {
        let source = r"def unprotected(flag, result):
    while True:
        if flag:
            break
        step()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def copied_tiny_exit(flag, result):
    while True:
        step()
        if flag:
            break
    return result

def generator(flag, result):
    while True:
        if flag:
            break
        yield step()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def tail_break(flag, result):
    while True:
        step()
        if flag:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def nested_terminal_break(flag, result, next_item, stop):
    while True:
        outer = next_item()
        if outer is None:
            break
        if flag:
            while True:
                item = next_item()
                if item is None:
                    break
                consume(item)
                if stop(item):
                    break
        first(result)
        second(result)
        third(result)
        fourth(result)
        fifth(result)
    return result

async def coroutine_terminal_break(result, read, stop):
    while True:
        item = await read()
        if item is None:
            break
        consume(item)
        if stop(item):
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def earlier_and_tail_break(first_flag, last_flag, result):
    while True:
        if first_flag:
            break
        step()
        if last_flag:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def protected_try_terminal(flag, result):
    try:
        while True:
            if flag:
                break
            step()
    except Exception:
        cleanup()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def protected_with_following(manager, flag, result):
    with manager:
        while True:
            if flag:
                break
            step()
        first(result)
        second(result)
        third(result)
        fourth(result)
        fifth(result)
    return result

";
        assert_matches_cpython_marshal(source, "constant_loop_break_borrowing.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_constant_loop_if_alternatives() {
        let source = r"def simple_final_if(flag, result):
    while True:
        step()
        if flag:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def final_if_with_fallthrough(result):
    while True:
        value = read()
        if value is None:
            break
        elif value is ignored:
            pass
        else:
            consume(value)
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result
";
        assert_matches_cpython_marshal(source, "terminal_constant_loop_if_alternatives.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_for_break_return_locations() {
        let source = r"def multiple_breaks(items, flag):
    if flag:
        for item in items:
            if first(item):
                break
            if second(item):
                work(item)
                break
    else:
        alternate()

def for_else(items, flag):
    if flag:
        for item in items:
            if first(item):
                work(item)
                break
        else:
            exhausted()
    else:
        alternate()

def break_then_fallthrough(items, flag):
    if flag:
        for item in items:
            if first(item):
                break
            work(item)
    else:
        alternate()

def nested_break_control(items, flag):
    if flag:
        for item in items:
            while ready(item):
                break
            work(item)
    else:
        alternate()

def no_break_control(items, flag):
    if flag:
        for item in items:
            work(item)
    else:
        alternate()

def guarded_match_break(items, flag):
    if flag:
        for item in items:
            match item:
                case _ if ready(item):
                    break
                case _:
                    work(item)
    else:
        alternate()

def direct_loop_control(items):
    for item in items:
        if first(item):
            work(item)
            break
";
        assert_matches_cpython_marshal(source, "terminal_for_break_return_locations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_copied_name_returns_after_non_simple_breaks() {
        let source = r"import dis

def final_else_name_return(first, second, result):
    while True:
        if first:
            step()
        elif second:
            other()
        else:
            break
    return result

def trailing_break_name_return(flag, result):
    while True:
        if flag:
            step()
            continue
        other()
        break
    return result

def final_else_call_return(first, second, result):
    while True:
        if first:
            step()
        elif second:
            other()
        else:
            break
    return wrap(result)

def result_load(function):
    loads = [
        instruction.opname
        for instruction in dis.get_instructions(function)
        if instruction.argval == 'result'
        and instruction.opname.startswith('LOAD_FAST')
    ]
    assert len(loads) == 1, loads
    return loads[0]

for function in (
    final_else_name_return,
    trailing_break_name_return,
    final_else_call_return,
):
    print(result_load(function))
";
        assert_matches_cpython_marshal(source, "non_simple_break_name_returns.py");
        assert_executes(source, "LOAD_FAST_BORROW\nLOAD_FAST_BORROW\nLOAD_FAST\n");
    }

    #[test]
    fn matches_cpython_load_ownership_for_constant_loop_exit_shapes() {
        let source = r"import dis

def break_inside_try(fp, size, result):
    data = b''
    while True:
        try:
            chunk = fp.read(size - len(data))
            data += chunk
            if len(chunk) == 0 or len(data) == size:
                break
        except BlockingIOError:
            pass
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def break_inside_with(lock, result):
    while True:
        with lock:
            if ready():
                break
        wait()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def breaking_fallthrough_alternative(first_flag, last_flag, result):
    while True:
        if first_flag:
            break
        else:
            step()
            if last_flag:
                break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def final_else_break(first_flag, second_flag, result):
    while True:
        if first_flag:
            first(result)
        elif second_flag:
            second(result)
        else:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def result_loads(function):
    return [
        instruction.opname
        for instruction in dis.get_instructions(function)
        if instruction.argval == 'result'
        and instruction.opname.startswith('LOAD_FAST')
    ]

for function in (
    break_inside_try,
    break_inside_with,
    breaking_fallthrough_alternative,
    final_else_break,
):
    print(','.join(result_loads(function)))
";
        assert_matches_cpython_marshal(source, "constant_loop_exit_shapes.py");
        assert_executes(
            source,
            "LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW\nLOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW\nLOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST_BORROW\nLOAD_FAST_BORROW,LOAD_FAST_BORROW,LOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST,LOAD_FAST\n",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_superinstruction_location_propagation() {
        let source = r#"def load_pair(first, second, item):
    consume(first, second, message=("Expected %s" % item))

def store_pair(items):
    return ["pair %s/%s" % (first, second) for first, second in items]

def reversed_store_pair(first, second):
    left, right = convert(first), convert(second)
    return "pair %s/%s" % (left, right)
"#;
        assert_matches_cpython_marshal(source, "superinstruction_locations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_borrowing_after_terminal_exception_handlers() {
        let source = "def returning(dependencies):\n    try:\n        before = load()\n        after = load()\n    except Exception:\n        return\n    for dependency in dependencies:\n        if dependency in after:\n            consume(dependency, before)\n\ndef reraising(dependencies):\n    try:\n        before = load()\n        after = load()\n    except Exception:\n        raise\n    for dependency in dependencies:\n        if dependency in after:\n            consume(dependency, before)\n";
        assert_matches_cpython_marshal(source, "terminal_handlers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_borrowing_after_terminating_assertion_handlers() {
        let source = r#"def following_assertion(client):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert False, f"unexpected error: {error}"
    assert response.status_code == 200


def multiple_assertions(client):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert False, f"unexpected error: {error}"
    assert response.status_code == 200
    assert response.json() == {"name": "value"}


def following_call(client, consume):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert False, f"unexpected error: {error}"
    consume(response)


def following_assignment(client):
    try:
        response = client.get("/thing")
    except Exception:
        assert 0, "unexpected error"
    status = response.status_code
    consume(status)


def dynamic_assertion_control(client, condition):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert condition, f"unexpected error: {error}"
    assert response.status_code == 200


def true_assertion_control(client):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert True, f"unexpected error: {error}"
    assert response.status_code == 200


def multiple_handler_control(client):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert False, f"unexpected error: {error}"
    except BaseException as error:
        consume(error)
    assert response.status_code == 200


def preceding_handler_statement_control(client, consume):
    try:
        response = client.get("/thing")
    except Exception as error:
        consume(error)
        assert False, f"unexpected error: {error}"
    return response.status_code


def exception_group_control(client):
    try:
        response = client.get("/thing")
    except* Exception as error:
        assert False, f"unexpected error: {error}"
    assert response.status_code == 200


def nested_name_return_control(client):
    try:
        try:
            response = client.get("/thing")
        except Exception as error:
            assert False, f"unexpected error: {error}"
        return response
    except LookupError:
        raise


def nested_attribute_return_control(client):
    try:
        try:
            response = client.get("/thing")
        except Exception as error:
            assert False, f"unexpected error: {error}"
        return response.status_code
    except LookupError:
        raise
"#;
        assert_matches_cpython_marshal(source, "terminating_assertion_handlers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_name_returns_after_terminal_exception_handlers() {
        let source = r"def direct():
    try:
        value = produce()
    except Exception:
        raise RuntimeError
    return value


def nested():
    try:
        try:
            value = produce()
        except SyntaxError:
            raise SyntaxError
        return value
    except Exception:
        raise
";
        assert_matches_cpython_marshal(source, "terminal_handler_name_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_fallthrough_handler_finally_borrows() {
        let source = r"def pass_handler(value):
    try:
        work()
    except Error:
        pass
    finally:
        consume(value)


def work_handler(value):
    try:
        work()
    except Error:
        recover()
    finally:
        consume(value)


def multi_statement_finally(value):
    try:
        work()
    except Error:
        recover()
    finally:
        prepare()
        consume(value)


def conditional_body(value, flag):
    try:
        if flag:
            work()
    except Error:
        recover()
    finally:
        prepare()
        consume(value)


def two_fallthrough_handlers(value):
    try:
        work()
    except ValueError:
        recover()
    except Error:
        other()
    finally:
        consume(value)


def handler_with_else(value):
    try:
        work()
    except Error:
        recover()
    else:
        success()
    finally:
        consume(value)


def no_handler(value):
    try:
        work()
    finally:
        consume(value)


def raise_then_fallthrough(value):
    try:
        work()
    except ValueError:
        raise
    except Error:
        recover()
    finally:
        consume(value)


def fallthrough_then_raise(value):
    try:
        work()
    except ValueError:
        recover()
    except Error:
        raise
    finally:
        consume(value)


def bare_handler(value):
    try:
        work()
    except:
        recover()
    finally:
        consume(value)
";
        assert_matches_cpython_marshal(source, "fallthrough_handler_finally_borrows.py");
    }

    #[test]
    fn uses_cpython_finally_load_kinds_at_handler_boundaries() {
        let source = r#"import dis


def fallthrough_handler(value):
    try:
        work()
    except Error:
        recover()
    finally:
        consume(value)


def compound_handler(value, flag):
    try:
        work()
    except Error:
        assert flag
        if flag:
            recover()
    finally:
        prepare()
        consume(value)


def conditional_body(value, flag):
    try:
        if flag:
            work()
    except Error:
        recover()
    finally:
        prepare()
        consume(value)


def multi_statement_finally(value):
    try:
        work()
    except Error:
        recover()
    finally:
        prepare()
        consume(value)


def conditional_finally(value, flag):
    try:
        work()
    except Error:
        recover()
    finally:
        if flag:
            consume(value)


def except_star_assignment_finally(value):
    try:
        work()
    except* Error:
        recover()
    finally:
        saved = value


def bare_handler(value):
    try:
        work()
    except:
        recover()
    finally:
        consume(value)


def loop_break_body(value, flag):
    try:
        while True:
            work()
            if flag:
                break
    except Error:
        recover()
    finally:
        consume(value)


def partial_raise_handler(value, flag):
    try:
        work()
    except Error:
        if flag:
            recover()
        else:
            raise
    finally:
        consume(value)


def value_loads(function):
    return " ".join(
        instruction.opname
        for instruction in dis.get_instructions(function)
        if instruction.argval == "value"
        and instruction.opname.startswith("LOAD_FAST")
    )


for function in (
    fallthrough_handler,
    compound_handler,
    conditional_body,
    multi_statement_finally,
    conditional_finally,
    except_star_assignment_finally,
    bare_handler,
    loop_break_body,
    partial_raise_handler,
):
    print(value_loads(function))
"#;
        assert_executes(
            source,
            "LOAD_FAST_BORROW LOAD_FAST\nLOAD_FAST_BORROW LOAD_FAST\nLOAD_FAST LOAD_FAST\nLOAD_FAST_BORROW LOAD_FAST\nLOAD_FAST_BORROW LOAD_FAST\nLOAD_FAST LOAD_FAST\nLOAD_FAST LOAD_FAST\nLOAD_FAST LOAD_FAST\nLOAD_FAST LOAD_FAST\n",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_handler_condition_exits() {
        let source = r"def unnamed(action, reraises):
    try:
        action()
    except Exception:
        if reraises:
            raise

def named(action, reraises):
    try:
        action()
    except Exception as error:
        if reraises(error):
            raise

def final_elif(action, first, second):
    try:
        action()
    except Exception:
        if first:
            raise
        elif second:
            raise
";
        assert_matches_cpython_marshal(source, "terminal_handler_condition_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_shared_terminal_handler_borrow_edges() {
        let source = "def branch_raise(flag):\n    if flag:\n        try:\n            action()\n        except Exception:\n            raise\n    else:\n        consume(flag)\n    after(flag)\n\ndef branch_return(flag):\n    if flag:\n        try:\n            action()\n        except Exception:\n            return\n    else:\n        consume(flag)\n    after(flag)\n\ndef nested_loop(values):\n    result = None\n    for item in values:\n        try:\n            result = load(item)\n        except KeyError:\n            raise ValueError(item)\n    return result\n";
        assert_matches_cpython_marshal(source, "shared_terminal_handler_edges.py");
    }

    #[test]
    fn matches_cpython_marshal_for_with_passes_and_folded_iterables() {
        let source = "def pass_before_loop(manager, values):\n    with manager as target:\n        pass\n        for value in values:\n            target.write(value)\n\ndef pass_in_loop_and_else(manager, values):\n    with manager as target:\n        for value in values:\n            pass\n            target.write(value)\n        else:\n            pass\n\ndef folded_iterable(manager):\n    with manager as target:\n        for value in (1,) if True else (2,):\n            target.write(value)\n\nwith (\n    manager\n) as target: pass\n";
        assert_matches_cpython_marshal(source, "with_passes.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_boolean_returns_through_with() {
        let source = r"def multiline_and(manager, value, expected):
    with manager:
        return (
            value is not None
            and value.kind == expected.kind
            and value.name == expected.name
            and value.flags == expected.flags
        )
";
        assert_matches_cpython_marshal(source, "with_multiline_boolean_return.py");
    }

    #[test]
    fn matches_cpython_marshal_for_assert_none_condition_owners_in_with_bodies() {
        let source = include_str!("fixtures/with_assert_none_condition_owners.py");
        assert_matches_cpython_marshal(source, "with_assert_none_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_iterator_condition_owners() {
        let source = include_str!("fixtures/protected_iterator_condition_owners.py");
        assert_matches_cpython_marshal(source, "protected_iterator_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synchronous_while_with_cleanup_spans() {
        let source = include_str!("fixtures/synchronous_while_with_cleanup_spans.py");
        assert_matches_cpython_marshal(source, "synchronous_while_with_cleanup_spans.py");
    }

    #[test]
    fn matches_cpython_marshal_for_pre_suspension_try_boolean_value_owners() {
        let source = include_str!("fixtures/pre_suspension_try_boolean_value_owners.py");
        assert_matches_cpython_marshal(source, "pre_suspension_try_boolean_value_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_loop_try_condition_owners() {
        let source = include_str!("fixtures/generator_loop_try_condition_owners.py");
        assert_matches_cpython_marshal(source, "generator_loop_try_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_generator_filter_dispatch_owners() {
        let source = include_str!("fixtures/nested_generator_filter_dispatch_owners.py");
        assert_matches_cpython_marshal(source, "nested_generator_filter_dispatch_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_iterator_tail_condition_owners() {
        let source = include_str!("fixtures/generator_iterator_tail_condition_owners.py");
        assert_matches_cpython_marshal(source, "generator_iterator_tail_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_iterator_tail_branch_owners() {
        let source = include_str!("fixtures/generator_iterator_tail_branch_owners.py");
        assert_matches_cpython_marshal(source, "generator_iterator_tail_branch_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_first_condition_owners() {
        let source = include_str!("fixtures/generator_first_condition_owners.py");
        assert_matches_cpython_marshal(source, "generator_first_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_protected_async_iterator_owners() {
        let source = include_str!("fixtures/protected_async_iterator_owners.py");
        assert_matches_cpython_marshal(source, "protected_async_iterator_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_post_try_fused_condition_owner() {
        let source = include_str!("fixtures/async_post_try.py");
        assert_matches_cpython_marshal(source, "async_post_try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_post_try_coroutine_condition_owners() {
        let source = include_str!("fixtures/post_try_coroutine_condition_owners.py");
        assert_matches_cpython_marshal(source, "post_try_coroutine_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unpreserved_handler_enclosing_try_owners() {
        let source = include_str!("fixtures/unpreserved_handler_enclosing_try_owners.py");
        assert_matches_cpython_marshal(source, "unpreserved_handler_enclosing_try_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_enclosing_try_owners() {
        let source = include_str!("fixtures/named_handler_enclosing_try_owners.py");
        assert_matches_cpython_marshal(source, "named_handler_enclosing_try_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_named_handler_finally_owner_slots() {
        let source = include_str!("fixtures/named_handler_finally_owner_slots.py");
        assert_matches_cpython_marshal(source, "named_handler_finally_owner_slots.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_handler_return_owners() {
        let source = include_str!("fixtures/nested_handler_return_owners.py");
        assert_matches_cpython_marshal(source, "nested_handler_return_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_ordinary_try_handler_return_owners() {
        let source = include_str!("fixtures/ordinary_try_handler_return_owners.py");
        assert_matches_cpython_marshal(source, "ordinary_try_handler_return_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_two_handlers_in_ordinary_try_generator() {
        let source = r"def f(value):
    yield value
    try:
        prepare(value)
        try:
            first(value)
        except FirstError:
            try:
                second(value)
            except SecondError as error:
                observe(error)
                return None
        return value
    except OuterError:
        return None
";
        assert_matches_cpython_marshal(source, "generator_inner_named_excluded.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_two_handlers_in_ordinary_try_async_generator() {
        let source = r"async def f(value):
    yield value
    try:
        await prepare(value)
        try:
            first(value)
        except FirstError:
            try:
                second(value)
            except SecondError as error:
                observe(error)
                return
        return
    except OuterError:
        return
";
        assert_matches_cpython_marshal(source, "async_generator_inner_named_excluded.py");
    }

    #[test]
    fn matches_cpython_marshal_for_two_named_handlers_inside_ordinary_try() {
        let source = r"async def f(value):
    try:
        await prepare(value)
        try:
            first(value)
        except FirstError as outer_error:
            try:
                second(value)
            except SecondError as error:
                observe(error)
                return None
        return value
    except OuterError:
        return None
";
        assert_matches_cpython_marshal(source, "coro_outer_named_excluded.py");
    }

    #[test]
    fn matches_cpython_marshal_for_coroutine_handler_return_through_older_iterator() {
        let source = r"async def f(values):
    try:
        for value in values:
            try:
                first(value)
            except FirstError:
                try:
                    return second(value)
                except SecondError:
                    observe(value)
    except OuterError:
        return None
";
        assert_matches_cpython_marshal(source, "coro_single_handler_iterator_excluded.py");
    }

    #[test]
    fn matches_cpython_marshal_for_manager_handler_enclosing_try_owners() {
        let source = include_str!("fixtures/manager_handler_enclosing_try_owners.py");
        assert_matches_cpython_marshal(source, "manager_handler_enclosing_try_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_iterator_handler_return_owners() {
        let source = include_str!("fixtures/iterator_handler_return_owners.py");
        assert_matches_cpython_marshal(source, "iterator_handler_return_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_post_return_boolean_owners() {
        let source = include_str!("fixtures/sync_with_post_return_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_post_return_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_return_join_boolean_owners() {
        let source = include_str!("fixtures/sync_with_return_join_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_return_join_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_return_join_gap_boolean_owners() {
        let source = include_str!("fixtures/sync_with_return_join_gap_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_return_join_gap_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_nonterminal_join_boolean_owners() {
        let source = include_str!("fixtures/sync_with_nonterminal_join_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_nonterminal_join_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_nonterminal_two_slot_boolean_owners() {
        let source = include_str!("fixtures/sync_with_nonterminal_two_slot_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_nonterminal_two_slot_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_simple_if_expression_owners() {
        let source = include_str!("fixtures/sync_with_simple_if_expression_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_simple_if_expression_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_post_inner_sync_with_boolean_owners() {
        let source = include_str!("fixtures/post_inner_sync_with_boolean_owners.py");
        assert_matches_cpython_marshal(source, "post_inner_sync_with_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_post_inner_try_boolean_owners() {
        let source = include_str!("fixtures/sync_with_post_inner_try_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_post_inner_try_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_post_return_or_condition_owners() {
        let source = include_str!("fixtures/sync_with_post_return_or_condition_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_post_return_or_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_post_return_or_two_condition_owners() {
        let source = include_str!("fixtures/sync_with_post_return_or_two_condition_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_post_return_or_two_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_iterator_simple_condition_owners() {
        let source = include_str!("fixtures/sync_with_iterator_simple_condition_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_iterator_simple_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_terminal_boolean_value_owners() {
        let source = include_str!("fixtures/sync_with_terminal_boolean_value_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_terminal_boolean_value_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_outer_try_boolean_owners() {
        let source = include_str!("fixtures/sync_with_outer_try_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_outer_try_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_nested_return_boolean_owners() {
        let source = include_str!("fixtures/sync_with_nested_return_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_nested_return_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_raise_join_boolean_owners() {
        let source = include_str!("fixtures/sync_with_raise_join_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_raise_join_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_return_generator_boolean_owners() {
        let source = include_str!("fixtures/sync_with_return_generator_boolean_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_return_generator_boolean_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_none_condition_owners() {
        let source = include_str!("fixtures/sync_with_none_condition_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_none_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_setup_cleanup_bridges() {
        for (filename, source) in [
            (
                "async_entry_five_sync_managers.py",
                include_bytes!("fixtures/async_entry_five_sync_managers.py").as_slice(),
            ),
            (
                "async_entry_fifteen_sync_managers.py",
                include_bytes!("fixtures/async_entry_fifteen_sync_managers.py").as_slice(),
            ),
        ] {
            assert_matches_cpython_marshal_bytes(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_async_with_handler_cleanup_owners() {
        let source = include_str!("fixtures/async_with_handler_cleanup_owners.py");
        assert_matches_cpython_marshal(source, "async_with_handler_cleanup_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_handler_condition_owners() {
        let source = include_str!("fixtures/handler_condition_owners.py");
        assert_matches_cpython_marshal(source, "handler_condition_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_exits_after_if_joins() {
        let source = "def retained(flag):\n    try:\n        action()\n    except Exception:\n        failed()\n    else:\n        if flag:\n            good()\n        else:\n            other()\n    after()\n\ndef ordinary(flag):\n    try:\n        action()\n    except Exception:\n        failed()\n    else:\n        if flag:\n            good()\n    after()\n";
        assert_matches_cpython_marshal(source, "try_if_join.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_if_trailing_try_exits() {
        let source = r"def nested_try(flag, outer):
    if outer:
        if flag:
            try:
                action()
            except Error:
                raise
    else:
        other()
    return None
";
        assert_matches_cpython_marshal(source, "nested_if_trailing_try.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_try_else_after_complete_if() {
        let source = r"def terminal_else(flag):
    try:
        if flag:
            action()
        else:
            other()
    except Error:
        failed()
    else:
        return None
    after()
";
        assert_matches_cpython_marshal(source, "terminal_try_else_complete_if.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_try_return_and_try_else_join_copies() {
        let source = r"def protected_branch_return(flag):
    try:
        if flag:
            try:
                action()
            except Error:
                raise ValueError
        return value
    except ValueError:
        return fallback


def try_else_nested_join(value, names, other):
    try:
        action()
    except TypeError:
        recover()
    else:
        if names is not None:
            if value.names is None:
                first()
            else:
                second()
        elif other:
            third()
    return value
";
        assert_matches_cpython_marshal(source, "nested_try_return_and_try_else_join.py");
    }

    #[test]
    fn matches_cpython_marshal_for_try_body_exits_after_if_joins() {
        let source = include_str!("fixtures/try_body_exit_joins.py");
        assert_matches_cpython_marshal(source, "try_body_exit_joins.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_try_if_exit_owners() {
        let source = r"def joined_loop(items, first, second):
    for item in items:
        if first:
            prefix = item
        else:
            prefix = None
        try:
            if second:
                value = item
            else:
                value = prefix
        except ValueError:
            value = None
        consume(value)


def nested_first_branch(items, outer, inner):
    for item in items:
        try:
            if outer:
                if inner:
                    value = item
                else:
                    value = None
            else:
                value = 0
        except ValueError:
            value = -1
        consume(value)


def nested_call_branch(first, second, rowtype, coltype, item):
    try:
        if first:
            row = rowtype(item)
        else:
            row = coltype(item)
        if second:
            result = row.dtype.char
        else:
            value = row + item
            if predicate(value):
                result = value.dtype.char
            else:
                result = type(value).dtype.char
    except ValueError:
        result = '!'
    except TypeError:
        result = '#'
    consume(result)


def try_else_control(items, flag):
    for item in items:
        try:
            if flag:
                value = item
            else:
                value = None
        except ValueError:
            value = 0
        else:
            consume(value)
";
        assert_matches_cpython_marshal(source, "direct_try_if_exit_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_numpy_testing_utils_try_owners() {
        let source = r"def tuple_boolean_marker(left, right):
    try:
        result = predicate(left) or predicate(right)
    except (ValueError, TypeError):
        result = False
    return result


def single_boolean_marker(left, right):
    try:
        result = predicate(left) or predicate(right)
    except ValueError:
        result = False
    return result


def assignment_handler(flag):
    try:
        if flag:
            value = first
        else:
            prepare()
            value = second
    except AttributeError:
        value = False
    consume(value)


def nonfinal_assignment_branch(flag):
    try:
        if flag:
            value = first
        else:
            prepare()
            value = second
        inspect(value)
    except AttributeError:
        value = False
    consume(value)


def sequential_tries(first, second):
    try:
        value = load(first)
        if first and second:
            if ready(value):
                return value
            else:
                raise ValueError(value)
    except (TypeError, ValueError):
        pass

    try:
        consume(value)
    except RuntimeError:
        pass
    return value
";
        assert_matches_cpython_marshal(source, "numpy_testing_utils_try_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_finally_iterator_break_synchronous_generator() {
        let source = r"def sync_generator(items):
    try:
        for item in items:
            if item:
                break
        yield item
    finally:
        cleanup()
";
        assert_matches_cpython_marshal(source, "finally_iterator_break_sync_generator.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_try_body_exit_nops() {
        let cases = [
            (
                "terminal_compare.py",
                r"def terminal_compare(value, name):
    try:
        if value <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return value
",
            ),
            (
                "terminal_unique_or.py",
                r"def terminal_unique_or(first, second, name):
    try:
        if not (first <= 0) or second <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return first
",
            ),
            (
                "multiple_false_edges.py",
                r"def multiple_false_edges(first, second, name):
    try:
        if first <= 0 and second <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return first
",
            ),
            (
                "break_not_retained.py",
                r"def break_not_retained(values):
    for value in values:
        try:
            if value:
                break
        except KeyError:
            break
    return None
",
            ),
            (
                "terminal_elif.py",
                r"def terminal_elif(first, second):
    try:
        if first <= 0:
            raise ValueError
        elif second <= 0:
            raise RuntimeError
    except TypeError:
        raise OSError from None
    return first
",
            ),
        ];

        for (filename, source) in cases {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_try_body_exit_nops_before_direct_exits_only() {
        let source = r"def before_return(flag):
    try:
        prepare()
        if flag:
            raise ValueError
    except TypeError:
        recover()
    return flag

def before_raise(flag):
    try:
        prepare()
        if flag:
            raise ValueError
    except TypeError:
        recover()
    raise RuntimeError

def before_conditional_return(values, required):
    for value in values:
        result = None
        try:
            result = prepare(value)
            if required and value is None:
                raise ValueError(value)
        except TypeError:
            recover(value)
        except LookupError:
            recover_again(value)
        if result is not None:
            return value
    return None
";
        assert_matches_cpython_marshal(source, "try_body_exit_before_direct_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_constant_true_comprehension_filter() {
        let source = "value: int\n[x for x in values if True]\n[x for x in values if not None]\n[result for x in values if not predicate]\n{target.attr: None for target.attr in values}\n[\n    result\n    for x in values\n    if True\n]\n[\n    result\n    for x in values\n    if x\n    if not None\n]\n[x for x in values if left and right]\n{x for x in values if lower < x < upper if accepted}\n(x for x in values if left and right if accepted)\n(x for x in values if left and right async for y in async_values if accepted)\n\ndef restored_then_subscript(values):\n    result = {}\n    result['key'] = [value for value in values]\n\ndef assignment_reuses_target():\n    tasks = [task for task in tasks]\n";
        assert_matches_cpython_marshal(source, "comprehension.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_tuple_not_nops() {
        let source = "multiline = (\n    not \"a\",\n    not \"b\",\n    (not \"c\",),\n)\nsame_line = (not \"a\", not \"b\")\nother_folds = (1 + 2, -3, ~4)\n";
        assert_matches_cpython_marshal(source, "folded_tuple.py");
    }

    #[test]
    fn matches_cpython_marshal_for_early_boolean_admissions_before_late_folds() {
        let source = r#"def straight():
    a = -1.0
    b = not 1
    c = not ""
    return a, b, c

def reversed():
    a = -1.0
    b = not ""
    c = not 1
    return a, b, c

def literal_first():
    a = False
    b = -1.0
    c = not 1
    d = not ""
    return a, b, c, d

def nested():
    a = -1.0
    b = (not 1, not "", not 2)
    return a, b

def linear():
    return (-1.0, not 1, not "")
"#;
        assert_matches_cpython_marshal(source, "early_boolean_admissions.py");
    }

    #[test]
    fn matches_cpython_marshal_for_early_boolean_admissions_in_copied_exits() {
        let source = r#"def copied_false_first(flag):
    if flag:
        pass
    else:
        x = not 1
    return not ""

def copied_true_first(flag):
    if flag:
        pass
    else:
        x = not ""
    return not 1
"#;
        assert_matches_cpython_marshal(source, "early_boolean_copied_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_early_boolean_admissions_with_discarded_owners() {
        let source = r#"def discarded_and():
    unused = 0 and not 1
    a = not ""
    b = not 1
    return a, b

def discarded_or():
    unused = 1 or not ""
    a = not 1
    b = not ""
    return a, b

def discarded_arm():
    unused = not 1 if 0 else 5
    a = not ""
    b = not 1
    return a, b
"#;
        assert_matches_cpython_marshal(source, "early_boolean_discarded_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_boolean_addition() {
        let source = "seed = 'seed'\nvalue = True + False\n";
        assert_matches_cpython_marshal(source, "folded_boolean_addition.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_mixed_numeric_multiplication() {
        let source = r"int_float = 65 * 1e9
float_int = 1e9 * 65
negative_int_float = -7 * 0.25
float_negative_int = 0.25 * -7
bool_float = True * 1.5
float_bool = 1.5 * False
signed_zero_left = -0.0 * 3
signed_zero_right = 3 * -0.0
overflow = 1e308 * 10
underflow = 1e-308 * 1e-308
";
        assert_matches_cpython_marshal(source, "folded_mixed_multiplication.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_mixed_numeric_power() {
        let source = r"int_float = 7 ** 0.5
float_int = 2.0 ** 53
int_negative = 10 ** -10
signed_base_negative = (-2) ** -3
float_base_integer = (-2.0) ** 3
underflow = 10 ** -100000
true_negative = True ** -1
zero_negative = 0 ** -1
float_overflow = 1e308 ** 2
";
        assert_matches_cpython_marshal(source, "folded_mixed_power.py");
    }

    #[test]
    fn matches_cpython_marshal_for_arbitrary_integer_constant_folding() {
        let source = r"positive_shift = 1 << 64
negative_shift = -1 << 64
maximum_shift = 1 << 127
maximum_negative_shift = -1 << 127
unfolded_shift = 1 << 128
nested_shift = (1 << 64) << 63
negative_nested_shift = (-1 << 64) << 63
zero_huge_shift = 0 << 999999999999999999999999999999999999999
positive_minus_small = 18446744073709551616 - 1
positive_plus_small = 18446744073709551616 + 1
unbounded_add = 999999999999999999999999999999999999999999999999999999999999 + 1
unbounded_subtract = 1000000000000000000000000000000000000000000000000000000000000 - 1
unbounded_cancel = 999999999999999999999999999999999999999999999999999999999999 + -999999999999999999999999999999999999999999999999999999999999
unbounded_nested_add = (1 << 127) + (1 << 127)
small_minus_positive = 1 - 18446744073709551616
negative_minus_small = -18446744073709551616 - 1
negative_plus_small = -18446744073709551616 + 1
below_i64 = -(1 << 63) - 1
above_i64 = -(1 << 63) + 1
signed_sum = -9223372036854775808 + -9223372036854775808
positive_product = (1 << 63) * 2
negative_product = (1 << 63) * -2
maximum_product = 18446744073709551615 * 9223372036854775807
unfolded_product = 18446744073709551616 * 9223372036854775808
zero_huge_product = 0 * 999999999999999999999999999999999999999
maximum_power = 2 ** 64
unfolded_power = 2 ** 65
unfolded_huge_power = 2 ** 100
zero_huge_power = 0 ** 999999999999999999999999999999999999999
complex_positive_power = [(1 + 2j) ** 3, (2 + 3j) ** 4]
complex_negative_power = [(1 + 2j) ** -2, (2 + 3j) ** -3]
real_divided_by_complex = [1 / (1 + 2j), 1 / (2 + 3j)]
complex_divided_by_real = [(-11 + 2j) / 125, (-46 - 9j) / 2197]
unfolded_complex_power = (1e308 + 1j) ** 2
unfolded_zero_negative_power = 0j ** -1
negative_right_shift = -3 >> 1
large_negative_right_shift = -18446744073709551617 >> 64
huge_positive_right_shift = 1 >> 999999999999999999999999999999999999999
huge_negative_right_shift = -1 >> 999999999999999999999999999999999999999
";
        assert_matches_cpython_marshal(source, "folded_arbitrary_integers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_large_constant_list() {
        let source = format!(
            "large = [{}]\n",
            (0..31)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_matches_cpython_marshal(&source, "large_constant_list.py");
    }

    #[test]
    fn matches_cpython_marshal_for_large_list_iterables() {
        let constants = (0..31)
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let dynamic = (0..31).map(|_| "value").collect::<Vec<_>>().join(", ");
        let source = format!(
            "def constant_loop():\n    for item in [{constants}]:\n        consume(item)\n\n\ndef dynamic_loop(value):\n    for item in [{dynamic}]:\n        consume(item)\n\n\ndef constant_contains(item):\n    return item in [{constants}]\n\n\ndef dynamic_contains(item, value):\n    return item in [{dynamic}]\n"
        );
        assert_matches_cpython_marshal(&source, "large_list_iterables.py");
    }

    #[test]
    fn matches_cpython_marshal_for_large_starred_collections() {
        for length in [30, 31, 32] {
            for starred_index in [0, 3, length - 1] {
                for element in ["value", "1"] {
                    let elements = (0..length)
                        .map(|index| {
                            if index == starred_index {
                                "*items"
                            } else {
                                element
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let source = format!(
                        "def build(value, items):\n    a = [{elements}]\n    b = ({elements},)\n    c = {{{elements}}}\n    return a, b, c\n"
                    );
                    assert_matches_cpython_marshal(&source, "large_starred_collections.py");
                }
            }
        }
    }

    #[test]
    fn matches_cpython_marshal_for_large_constant_set_iterables() {
        for length in [30, 31, 32] {
            let elements = (0..length)
                .map(|index| format!("'value{index}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let source = format!(
                "def check(value):\n    present = value in {{{elements}}}\n    for item in {{{elements}}}:\n        consume(item)\n    return present\n"
            );
            assert_matches_cpython_marshal(&source, "large_constant_set_iterables.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_large_unpacked_dictionaries() {
        for length in [15, 16, 17, 18, 34] {
            for unpack_index in [0, 15, length] {
                let mut items = (0..length)
                    .map(|index| format!("{index}: value"))
                    .collect::<Vec<_>>();
                items.insert(unpack_index, "**mapping".to_string());
                let items = items.join(", ");
                let source = format!("def build(value, mapping):\n    return {{{items}}}\n");
                assert_matches_cpython_marshal(&source, "large_unpacked_dictionaries.py");
            }
        }
    }

    #[test]
    fn matches_cpython_marshal_for_multiline_large_keyword_calls() {
        for count in [15, 16, 17] {
            for (prefix, suffix) in [
                ("", ""),
                ("        *args,\n", ""),
                ("        **mapping,\n", ""),
                ("", "        **mapping,\n"),
            ] {
                for callee in ["consume", "obj.method"] {
                    let mut keywords = String::new();
                    for index in 0..count {
                        writeln!(&mut keywords, "        k{index}=value.attr,").unwrap();
                    }
                    let source = format!(
                        "def f(consume, obj, value, args, mapping):\n    return outer({callee}(\n{prefix}{keywords}{suffix}    ))\n"
                    );
                    assert_matches_cpython_marshal(&source, "multiline_large_keyword_calls.py");
                }
            }
        }
    }

    #[test]
    fn matches_cpython_marshal_for_large_keyword_call_conditional_returns() {
        let keywords = (0..16)
            .map(|index| format!("p{index}=value"))
            .collect::<Vec<_>>()
            .join(", ");
        for expression in ["make() if flag else None", "value if flag else make()"] {
            let source = format!(
                "def build(flag, value):\n    return construct({keywords}, final={expression})\n"
            );
            assert_matches_cpython_marshal(&source, "large_keyword_call_conditional_returns.py");
        }
    }

    #[test]
    fn matches_cpython_marshal_for_a_terminal_finally_suite() {
        let source = "\"\"\"doc\"\"\"\ntry:\n    raise ValueError\nfinally:\n    raise RuntimeError\nafter = 1\ntry:\n    action()\nexcept Error as captured:\n    consume(captured)\n";
        assert_matches_cpython_marshal(source, "terminal_finally.py");
    }

    #[test]
    fn matches_cpython_marshal_for_annotation_thunk_edges() {
        let source = "from typing import Annotated, List, TypedDict\n\n()\nvalue: int\n\ndef marker():\n    pass\n\nclass Item(TypedDict):\n    nodes: List[TypedDict('Node', {'name': str})]\n\nclass Metadata:\n    value: Annotated[int, marker()]\n\ndef outer():\n    from pathlib import Path\n\n    class Visitor:\n        def visit(self, value: Path) -> None:\n            pass\n";
        assert_matches_cpython_marshal(source, "annotation.py");
    }

    #[test]
    fn matches_cpython_marshal_for_fresh_annotation_locals_and_explicit_interns() {
        let source = r"def consume(**kwargs):
    return kwargs

class Container:
    value: (consume(format=int), consume(méthode=str))
";
        assert_matches_cpython_marshal(source, "marshal_hints.py");
    }

    #[test]
    fn matches_cpython_marshal_for_annotations_in_compound_suites() {
        let source = r"with manager:
    in_with: WithAnnotation = 1

try:
    in_try: TryAnnotation = 2
except Error:
    in_except: ExceptAnnotation = 3
else:
    in_else: ElseAnnotation = 4
finally:
    in_finally: FinallyAnnotation = 5

match subject:
    case 1:
        in_match: MatchAnnotation = 6
";
        assert_matches_cpython_marshal(source, "compound_annotations.py");
    }

    #[test]
    fn matches_cpython_marshal_for_unreachable_deferred_annotations() {
        let source =
            "seed = 0\nraise terminal\nvalue: Annotation = build()\ndeclaration: Annotation\n";
        assert_matches_cpython_marshal(source, "unreachable_annotation.py");
    }

    #[test]
    fn matches_cpython_marshal_for_an_inline_constant_with_body() {
        let source = "import io\nwith (\n    io  # comment\n    .open('file.txt') as file\n): ...\nwith manager:\n    while condition:\n        body()\n";
        assert_matches_cpython_marshal(source, "with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_tuple_containing_a_constant_slice() {
        let source = "constant = lambda value: value[:, 1]\ndynamic = lambda value: value[::-1]\nwith_defaults = lambda value=1, other=2: value\nstring_slice = 'héllo'[1:4]\nstepped_string_slice = 'abcdef'[1:5:2]\nboolean_slice = 'abc'[False:True:True]\nbytes_slice = b'prefix'[2:]\ntuple_slice = (1, 2, 3)[1:]\nnegative_slice = 'abc'[:-1]\nstring_index = 'héllo'[1]\nfstring_index = f'abc'[0]\nbytes_index = b'prefix'[-1]\ntuple_index = (1, 2, 3)[-2]\ndynamic_list_index = [1, 2, 3][0]\nmember = item in [1, 2]\ndouble_negation = not not item\n5.0 ** 5.0\nTrue ** True\nFalse ** False\n() << 0 ** 99999999999999999999999999\n1 - 2\ninverted = ~1\nnegative_zero = -0000\nmixed = 1 + 2.5\nassert (False, 'x')\nassert (False,)\nassert ()\nassert True\npacked = fn('User', **{'name': str})\nunicode_keyword = fn(café=1)\nhuge = -999999999999999999999999999999999999999999\nadjacent = ('' f'prefix {item}')\nleft if condition else right\n'%s' % ('value',)\nif (1, 2):\n    pass\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_constant_sequence_slice_bound_folding_order() {
        for sequence in ["'abcd'", "'aébc'", "b'abcd'", "(1, 2, 3, 4)"] {
            for slice in [
                ":0 + 1",
                "-0:",
                "0 + 1:",
                ":(1 if True else 2)",
                ":(1,)[0]",
                ":+1",
                "not True:",
                "::0 + 1",
                "0 + 1::1",
                ":1",
                "0:",
                "::1",
                ":True",
            ] {
                let source = format!("def f():\n    return {sequence}[{slice}]\n");
                assert_matches_cpython_marshal(&source, "constant_sequence_slice_bounds.py");
            }
        }
    }

    #[test]
    fn matches_cpython_marshal_for_shared_traversal_containers() {
        let source = r#"def outer():
    def nested(value):
        return (
            value in {("alpha", 1), ("beta", 2)},
            value[257:256:2],
            ((1, 2), (1, 2)),
        )

    return nested
"#;
        assert_matches_cpython_marshal(source, "marshal_traversal.py");
    }

    #[test]
    fn matches_cpython_marshal_for_lambda_edge_cases() {
        let source = "from __future__ import barry_as_FLUFL, generator_stop, print_function\nsimple = lambda: (yield None)\nyield_value = lambda value: (yield value)\ndelegating = lambda: (yield from source())\ndef eliminated():\n    while False:\n        value = lambda: missing\ndef   trailing_semicolon():\n    value = 1 \\\n        ;\ndef trailing_semicolon_try():\n    try:\n        value = 1;\n    except ValueError:\n        pass;\ndef overridden_return():\n    try:\n        return 1\n    finally:\n        return 2\ndef overridden_handler_return():\n    try:\n        pass\n    except:\n        return 1\n    finally:\n        return 2\ndef overridden_try_and_handler_returns():\n    try:\n        call()\n        return 1\n    except:\n        return 2\n    finally:\n        return 3\ndef assigned_after_try_except_finally():\n    value = None\n    try:\n        call()\n        value = 1\n    except:\n        value = 2\n    finally:\n        value = 3\n    return value\ndef empty_try_else():\n    try:\n        pass\n    except:\n        return 1\n    else:\n        return 2\ndef dead_loop_return(value):\n    while value > 0:\n        break\n        return 1\ndef return_inside_with():\n    with manager:\n        return 1\ndef finally_return_in_loop(items):\n    for item in items:\n        try:\n            use(item)\n        finally:\n            return\nasync  def terminal_async_comprehension():\n    if test:\n        values = [value async for value in source]\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_return_through_finally() {
        let source = "def return_through_finally(items):\n    for item in items:\n        try:\n            pass\n        except Exception:\n            return\n        finally:\n            consume(item)\n\ndef preserve_return_value():\n    try:\n        pass\n        return produce()\n    finally:\n        consume()\n\ndef preserve_try_pass():\n    try:\n        pass\n        return produce()\n    except Exception:\n        recover()\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_return_unwind_owners() {
        let source = r"def loop_value(items):
    try:
        for item in items:
            try:
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                return item
            except Error:
                pass
    finally:
        cleanup()
    return None


def literal_values(manager):
    try:
        with manager:
            prepare()
        try:
            action()
            return True
        except Error:
            return False
    finally:
        cleanup()


def nested_handler(compiler):
    try:
        command = compiler.compiler
    except AttributeError:
        try:
            if not compiler.initialized:
                compiler.initialize()
        except (SetupError, ValueError):
            return False
        command = [compiler.cc]
    return command
";
        assert_matches_cpython_marshal(source, "nested_return_unwinds.py");
    }

    #[test]
    fn matches_cpython_marshal_for_loop_tail_boolean_expression_exits() {
        let source = r"def conjunction(items):
    for item in items:
        left(item) and right(item)


def disjunction(items):
    for item in items:
        first(item) or second(item) or third(item)


def non_tail(items):
    for item in items:
        left(item) and right(item)
        consume(item)


def final_call(items):
    for item in items:
        consume(item)


def assigned(items):
    for item in items:
        result = left(item) and right(item)
";
        assert_matches_cpython_marshal(source, "loop_tail_boolean_exits.py");
    }

    #[test]
    fn matches_cpython_marshal_for_direct_nested_handler_return_owners() {
        let source = r"def direct_none(action):
    try:
        outer()
    except TypeError:
        try:
            action()
            raise AssertionError()
        except TypeError:
            return


def direct_value(action):
    try:
        outer()
    except TypeError:
        try:
            action()
            raise AssertionError()
        except ValueError:
            return False
        except TypeError:
            return True


def top_level(action):
    try:
        action()
    except TypeError:
        return


def nested_fallthrough(action):
    try:
        outer()
    except TypeError:
        try:
            action()
        except TypeError:
            recover()
";
        assert_matches_cpython_marshal(source, "nested_handler_return_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_finally_end_control_flow() {
        let source = "def conditional_return(flag):\n    try:\n        pass\n    finally:\n        if flag:\n            return\n\ndef nested_return():\n    try:\n        try:\n            pass\n        finally:\n            return\n    finally:\n        pass\n\ndef nested_handler_return():\n    try:\n        pass\n    finally:\n        try:\n            return\n        except Exception:\n            pass\n\ndef break_from_finally():\n    while True:\n        try:\n            pass\n        finally:\n            break\n\ndef continue_from_finally():\n    while True:\n        try:\n            pass\n        finally:\n            continue\n";
        assert_matches_cpython_marshal(source, "finally_end.py");
    }

    #[test]
    fn matches_cpython_marshal_for_pass_branches_and_surrogates() {
        let source = "def pass_elif(value):\n    if value:\n        pass\n    elif check(value):\n        value = update(value)\n    return value\n\ndef surrogates():\n    explicit_pair = \"\\ud800\\udc00heythere\"\n    lone_surrogate = \"\\ud800\"\n    mixed = \"\u{fffd}\\ud800\"\n    scalar = \"\\U00010000heythere\"\n    return explicit_pair, lone_surrogate, mixed, scalar\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_a_percent_formatted_fstring() {
        let source = "f\"hello %s %s\" % (1, 2)\n\"%s %s\" % (1, 2)\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn matches_cpython_marshal_for_surrogates_in_folded_containers() {
        let source = r#"def tuple_argument(function):
    return function(("\uddf8",))

def iterated_values():
    for value in ["\ud800", ("\udfff",), "\ufffd"]:
        consume(value)
"#;
        assert_matches_cpython_marshal(source, "nested_surrogates.py");
    }

    #[test]
    fn matches_cpython_marshal_for_multiplied_surrogate_constants() {
        let source = r#"def left():
    return 100 * "\udc00"

def right():
    return "\udc00" * 100

def mixed():
    return 2 * ("\ufffd" "\udc00")

def zero():
    return 0 * "\udc00"

def negative():
    return -1 * "\udc00"

def at_limit():
    return 4096 * "\udc00"

def over_limit():
    return 4097 * "\udc00"

def two_char_limit():
    return 2048 * "a\udc00"

def two_char_over_limit():
    return 2049 * "a\udc00"

def nested():
    return 2 * (3 * "\udc00")
"#;
        assert_matches_cpython_marshal(source, "multiplied_surrogates.py");
    }

    #[test]
    fn matches_cpython_marshal_for_source_aware_surrogate_folds() {
        for source in [
            r#"value = "\ud800"[0]"#,
            r#"value = "\ud800x"[-1]"#,
            r#"value = "a\ud800b\udfff"[1:4:2]"#,
            r#"value = "a\ud800b"[:1]"#,
            r#"value = "\ud800"[:0]"#,
            r#"value = ("\ud800",)[0]"#,
            r#"value = ("\ud800", "x")[:1]"#,
            r#"value = ("\ud800",) + ("x",)"#,
            r#"value = ("\ud800",) * 2"#,
            r#"value = 2 * (("\ud800",),)"#,
            r#"value = (f"\ud800",)"#,
            r#"value = f"\ud800" + "x""#,
            r#"value = (f"\ud800" f"\udfff")[1]"#,
            r#"value = (rf"\ud800" f"\udfff")[-1]"#,
            r#"value = item["\ud800":]"#,
            r#"value = item[:"\ud800":]"#,
            r#"value = item[::"\ud800"]"#,
            r#"value = item[r"\ud800" "\udfff":]"#,
            r#"item["\ud800":] = value"#,
            r#"del item["\ud800":]"#,
        ] {
            assert_matches_cpython_marshal(source, "surrogate_folds.py");
        }
    }

    #[test]
    fn executes_source_aware_surrogate_folds() {
        assert_executes(
            r#"class Echo:
    def __getitem__(self, key):
        return key

print(ord("\ud800"[0]))
print(ord((("\ud800",) * 2)[1]))
print(ord((f"\ud800" + "x")[0]))
print(ord(Echo()["\ud800":].start))
"#,
            "55296\n55296\n55296\n55296\n",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_surrogate_fstring_literal_segments() {
        let source = r#"def regular(first, second):
    return f"\ud83d{first}\udc0d{second}\udfff"

def replacement_order(value):
    return f"\ufffd\ud800{value}\udfff\ufffd"

def raw_then_regular(value):
    return fr"\ud800 \ufffd {value}" f"\udfff"

def regular_then_raw(value):
    return f"\ud800{value}" rf"\udfff \ufffd"

def raw_actual_replacement(value):
    return rf"\ud800 �{value}"

def plain_then_fstring(value):
    return "\ud83d" f"{value}\udc0d"

def format_spec(value):
    return f"{value:\ud800}"

def dynamic_format_spec(value, width):
    return f"{value:\ud800{width}\udfff}"

def raw_format_spec(value, width):
    return rf"{value:\ud800 �{width}}"

def adjacent_regular():
    return f"\ud800" f"\udfff"

def adjacent_raw_then_regular():
    return rf"\ud800 �" f"\udfff"

def adjacent_regular_then_raw():
    return f"\ud800" rf"\udfff �"

def adjacent_plain_then_fstring():
    return "\ud800" f"\udfff"

def large(value):
    return f"\ud800{value}a{value}b{value}c{value}d{value}e{value}f{value}g{value}h{value}i{value}j{value}k{value}l{value}m{value}n{value}o{value}p\udfff"
"#;
        assert_matches_cpython_marshal(source, "surrogate_fstring_segments.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synthesized_percent_format_target_locations() {
        let source = r#"def terminal_false_target(flag, value):
    if flag:
        return value
    return "tail=%s" % value


def else_target(flag, value):
    if flag:
        result = "left=%s" % value
    else:
        result = "right=%s" % value
    return result


def final_elif_target(first, second, value):
    if first:
        return first
    elif second:
        return second
    return "tail=%s" % value


def entry_block(value):
    return "entry=%s" % value


def true_fallthrough(flag, value):
    if flag:
        return "body=%s" % value
    return value


def formatted_first(flag, value):
    if flag:
        return flag
    return "%s=tail" % value


def filtered_generator(items):
    return "".join(" %s=%s" % (key, value) for key, value in items if value)
"#;
        assert_matches_cpython_marshal(source, "percent_format_target.py");
    }

    #[test]
    fn matches_cpython_marshal_for_synthesized_percent_format_fallthrough_locations() {
        let source = r#"def named_handler(callback, value):
    try:
        return callback()
    except Exception as error:
        return "error=%s" % (error,)


def bare_handler(callback, value):
    try:
        return callback()
    except Exception:
        return "error=%s" % (value,)


def try_else(obj, value):
    try:
        name = obj.name
    except AttributeError:
        return value
    else:
        return "name=%s" % (name,)


def entry_block(value):
    return "entry=%s" % (value,)


def preceded_in_handler(callback, value):
    try:
        return callback()
    except Exception:
        result = value
        return "error=%s" % (result,)


def shared_join(flag, left, right):
    if flag:
        value = left
    else:
        value = right
    return "value=%s" % (value,)
"#;
        assert_matches_cpython_marshal(source, "percent_format_fallthrough.py");
    }

    #[test]
    fn matches_cpython_marshal_for_masked_array_control_flow() {
        let source = r#"def flatten(sequence):
    try:
        for element in sequence:
            if hasattr(element, "__iter__"):
                yield from flatten(element)
            else:
                yield element
    except TypeError:
        yield sequence


def view(dtype, type_, output, fill):
    if dtype is None:
        output = convert(dtype)
    elif type_ is None:
        try:
            if issubclass(dtype, object):
                output = convert(dtype)
                dtype = None
            else:
                output = convert(dtype)
        except TypeError:
            output = convert(dtype)
    else:
        output = convert(dtype)

    if output is not None:
        if fill is None:
            if dtype is None:
                pass
            else:
                output.fill = None
        else:
            output.fill = fill
    return output


def filled(action, self, mask, result):
    try:
        action()
    except (TypeError, AttributeError):
        result = fallback()
    except IndexError:
        if self.data.shape:
            raise
        elif mask:
            result = replacement()
        else:
            result = self.data
    return result
"#;
        assert_matches_cpython_marshal(source, "masked_array_control_flow.py");
    }
    #[test]
    fn matches_cpython_marshal_for_folded_not_and_terminal_while() {
        let source = "folded = not (\n    True\n)\n\ndef loop():\n    while not check() and left < right:\n        pass\n";
        assert_matches_cpython_marshal(source, "example.py");
    }

    #[test]
    fn executes_arithmetic_names_and_calls() {
        assert_executes("answer = 6 * 7\nprint(answer)\n", "42\n");
    }

    #[test]
    fn executes_copied_finally_handler_returns() {
        let source = include_str!("fixtures/execution/copied_finally_handler_returns.py");
        assert_executes(source, "ok\n");
    }

    #[test]
    fn matches_cpython_marshal_for_copied_finally_outer_iterator() {
        for (filename, source) in [
            (
                "outer_iterator_copied_return_none_target.py",
                r#"async def run(items, resource):
    for item in items:
        try:
            value = await obtain(item)
            return finish(value).result
        except Error as error:
            if retry(error):
                await pause()
        finally:
            if resource is not None:
                close = getattr(resource, "close", None)
                if close is not None:
                    await close()
"#,
            ),
            (
                "outer_iterator_copied_return_none_no_call_control.py",
                r#"async def run(items, resource):
    for item in items:
        try:
            value = await obtain(item)
            return value.result
        except Error as error:
            if retry(error):
                await pause()
        finally:
            if resource is not None:
                close = getattr(resource, "close", None)
                if close is not None:
                    await close()
"#,
            ),
            (
                "outer_iterator_copied_return_none_two_calls_control.py",
                r#"async def run(items, resource):
    for item in items:
        try:
            value = await obtain(item)
            return finish(convert(value)).result
        except Error as error:
            if retry(error):
                await pause()
        finally:
            if resource is not None:
                close = getattr(resource, "close", None)
                if close is not None:
                    await close()
"#,
            ),
            (
                "outer_iterator_copied_return_none_sync_control.py",
                r#"def run(items, resource):
    for item in items:
        try:
            value = obtain(item)
            return finish(value).result
        except Error as error:
            if retry(error):
                pause()
        finally:
            if resource is not None:
                close = getattr(resource, "close", None)
                if close is not None:
                    close()
"#,
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_copied_finally_outer_async_manager_membership() {
        for (filename, source) in [
            (
                "membership_return_4_global_calls_target.py",
                r#"async def run(manager, resource, result, selected):
    async with manager:
        try:
            if selected:
                tick()
                tick()
                tick()
                tick()
                return result
            else:
                return None
        except Error as error:
            report(error)
            return None
        finally:
            if "resource" in locals():
                try:
                    await resource.close()
                except Error:
                    report_failure()
"#,
            ),
            (
                "membership_return_3_global_calls_upper_owned_control.py",
                r#"async def run(manager, resource, result, selected):
    async with manager:
        try:
            if selected:
                tick()
                tick()
                tick()
                return result
            else:
                return None
        except Error as error:
            report(error)
            return None
        finally:
            if "resource" in locals():
                try:
                    await resource.close()
                except Error:
                    report_failure()
"#,
            ),
            (
                "membership_return_6_global_calls_lower_owned_control.py",
                r#"async def run(manager, resource, result, selected):
    async with manager:
        try:
            if selected:
                tick()
                tick()
                tick()
                tick()
                tick()
                tick()
                return result
            else:
                return None
        except Error as error:
            report(error)
            return None
        finally:
            if "resource" in locals():
                try:
                    await resource.close()
                except Error:
                    report_failure()
"#,
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_folded_value_borrow_boundaries() {
        for (filename, source) in [
            (
                "folded_value_else_none_or_attribute.py",
                r"def choose(flag, primary, fallback):
    value = primary if flag else None or fallback.value
    consume(value)
",
            ),
            (
                "folded_value_else_none_or_chain.py",
                r"def choose(flag, primary, fallback):
    value = primary if flag else None or fallback.value or default()
    consume(value)
",
            ),
            (
                "folded_value_after_if_expression_join.py",
                r"def choose(flag, enabled, tail):
    value = build(select() if flag else other(), enabled=False or enabled)
    consume(tail, value)
",
            ),
            (
                "folded_value_no_constant_control.py",
                r"def choose(flag, primary, fallback):
    value = primary if flag else fallback.value
    consume(value)
",
            ),
            (
                "folded_value_nonempty_prefix_control.py",
                r"def choose(flag, primary, prefix, fallback):
    value = primary if flag else combine(prefix, None or fallback.value)
    consume(value)
",
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_false_assert_handler_finalizers() {
        for (filename, source) in [
            (
                "false_assert_handler_async.py",
                r"async def run(resource, value):
    result = None
    try:
        result = await resource.work(value)
    except Error as error:
        message = str(error)
        report(message)
        assert False
    finally:
        await resource.close()
    return convert(result)
",
            ),
            (
                "false_assert_handler_sync_control.py",
                r"def run(resource, value):
    result = None
    try:
        result = resource.work(value)
    except Error as error:
        message = str(error)
        report(message)
        assert False
    finally:
        resource.close()
    return convert(result)
",
            ),
            (
                "raise_handler_async_control.py",
                r"async def run(resource, value):
    result = None
    try:
        result = await resource.work(value)
    except Error as error:
        message = str(error)
        report(message)
        raise Error
    finally:
        await resource.close()
    return convert(result)
",
            ),
            (
                "runtime_assert_handler_async_control.py",
                r"async def run(resource, value):
    result = None
    try:
        result = await resource.work(value)
    except Error as error:
        message = str(error)
        report(message)
        assert value
    finally:
        await resource.close()
    return convert(result)
",
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_nested_terminal_handler_else() {
        for (filename, source) in [
            (
                "nested_terminal_else_target.py",
                r"def run(factory, first, second, value, consume):
    try:
        value = factory()
    except Error:
        if first:
            value = {}
        else:
            if second:
                value = {}
            else:
                raise
    consume(value)
    return value
",
            ),
            (
                "nested_terminal_else_suffix_control.py",
                r"def run(factory, first, second, value, consume):
    try:
        value = factory()
    except Error:
        if first:
            value = {}
        else:
            if second:
                value = {}
            else:
                raise
            consume(second)
    consume(value)
    return value
",
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_nested_handler_manager_arms() {
        for (filename, source) in [
            (
                "nested_manager_arm_target.py",
                r"def run(manager, other, flag, value, work):
    try:
        work(value)
    except Error:
        if flag:
            with manager:
                with other:
                    work(value)
        else:
            work(value)
    try:
        work(value)
    except Error:
        pass
    work(value)
    return value
",
            ),
            (
                "nested_manager_arm_suffix_control.py",
                r"def run(manager, other, flag, value, work):
    try:
        work(value)
    except Error:
        if flag:
            with manager:
                with other:
                    work(value)
            work(value)
        else:
            work(value)
    try:
        work(value)
    except Error:
        pass
    work(value)
    return value
",
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_worker_recovery_finally_cleanup() {
        for (filename, source) in [
            (
                "async_conditional_recovery.py",
                r"async def worker(queue, process):
    while True:
        item = await queue.get()
        await prepare()
        try:
            if item is None:
                return
            try:
                result = await process(item.value)
            except Error as error:
                if not item.done():
                    item.set_error(error)
            else:
                if not item.done():
                    item.set_result(result)
        finally:
            queue.task_done()
",
            ),
            (
                "sync_nested_recovery.py",
                r#"def worker(queue, output, metric, logger, sentinel):
    while True:
        item = queue.get()
        try:
            if item is sentinel:
                queue.put(sentinel)
                return
            try:
                prepare(item)
                result = item.convert()
                output.put(result)
            except Error:
                logger.report(item)
                try:
                    metric.update(item)
                except Error:
                    logger.report("failed")
        finally:
            queue.task_done()
"#,
            ),
            (
                "async_populated_suffix_control.py",
                r"async def worker(queue, process):
    while True:
        item = await queue.get()
        await prepare()
        try:
            if item is None:
                return
            try:
                result = await process(item.value)
            except Error as error:
                if not item.done():
                    item.set_error(error)
            else:
                if not item.done():
                    item.set_result(result)
            record(item)
        finally:
            queue.task_done()
",
            ),
            (
                "sync_populated_suffix_control.py",
                r#"def worker(queue, output, metric, logger, sentinel):
    while True:
        item = queue.get()
        try:
            if item is sentinel:
                queue.put(sentinel)
                return
            try:
                prepare(item)
                result = item.convert()
                output.put(result)
            except Error:
                logger.report(item)
                try:
                    metric.update(item)
                except Error:
                    logger.report("failed")
            record(item)
        finally:
            queue.task_done()
"#,
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_protected_async_surviving_break() {
        let source = r"async def run(manager, source, value):
    async with manager:
        try:
            async for item in source:
                if selected(item):
                    break
                consume(item)
        except Error:
            pass
        consume(value)
        consume(source)
";
        assert_matches_cpython_marshal(source, "protected_async_surviving_break_control.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_finally_while_controls() {
        for (filename, source) in [
            (
                "finally_terminal_while_multi_false.py",
                r"def run(service, release, clock):
    try:
        start(service)
    finally:
        release()
        deadline = clock() + 2
        while service.pending and clock() < deadline:
            pause()
    assert service.pending == {}
",
            ),
            (
                "finally_terminal_while_suffix_control.py",
                r"def run(service, release, clock):
    try:
        start(service)
    finally:
        release()
        deadline = clock() + 2
        while service.pending and clock() < deadline:
            pause()
        complete()
    assert service.pending == {}
",
            ),
            (
                "finally_terminal_while_break_control.py",
                r"def run(service, release, clock):
    try:
        start(service)
    finally:
        release()
        deadline = clock() + 2
        while service.pending and clock() < deadline:
            if stop():
                break
            pause()
    assert service.pending == {}
",
            ),
        ] {
            assert_matches_cpython_marshal(source, filename);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_copied_finally_prefix_owners() {
        let source = include_str!("fixtures/copied_finally_prefix_owners.py");
        assert_matches_cpython_marshal(source, "copied_finally_prefix_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_bare_return_finally_owners() {
        let source = include_str!("fixtures/bare_return_finally_owners.py");
        assert_matches_cpython_marshal(source, "bare_return_finally_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_post_comprehension_boolean_value_owners() {
        let source = include_str!("fixtures/post_comprehension_boolean_value_owners.py");
        assert_matches_cpython_marshal(source, "post_comprehension_boolean_value_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_context_manager_handler_returns() {
        let source = include_str!("fixtures/context_manager_handler_returns.py");
        assert_matches_cpython_marshal(source, "context_manager_handler_returns.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_wrapped_boolean_value_owners() {
        let source = include_str!("fixtures/sync_with_wrapped_boolean_value_owners.py");
        assert_matches_cpython_marshal(source, "sync_with_wrapped_boolean_value_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_generator_terminal_while_break_owners() {
        let source = include_str!("fixtures/generator_terminal_while_break_owners.py");
        assert_matches_cpython_marshal(source, "generator_terminal_while_break_owners.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_conditional_expression_returns() {
        let source = include_str!("fixtures/nested_conditional_expression_returns.py");
        assert_matches_cpython_marshal(source, "nested_conditional_expression_returns.py");
    }

    #[test]
    fn matches_cpython_marshal_for_sync_with_split_return_handlers() {
        let source = include_str!("fixtures/sync_with_split_return_handlers.py");
        assert_matches_cpython_marshal(source, "sync_with_split_return_handlers.py");
    }

    #[test]
    fn matches_cpython_marshal_for_iterator_with_return_unwind() {
        let source = include_str!("fixtures/iterator_with_return_unwind.py");
        assert_matches_cpython_marshal(source, "iterator_with_return_unwind.py");
    }

    #[test]
    fn matches_cpython_marshal_for_terminal_while_break_normalization() {
        let source = include_str!("fixtures/terminal_while_break_normalization.py");
        assert_matches_cpython_marshal(source, "terminal_while_break_normalization.py");
    }

    #[test]
    fn matches_cpython_marshal_for_nested_iterator_tail_conditions() {
        let source = include_str!("fixtures/nested_iterator_tail_conditions.py");
        assert_matches_cpython_marshal(source, "nested_iterator_tail_conditions.py");
    }

    #[test]
    fn matches_cpython_marshal_for_async_iterator_manager_tail_break_owners() {
        for (name, source) in [
            (
                "manager_tail_break_target.py",
                include_str!("fixtures/borrow_finally_reachability/manager_tail_break_target.py"),
            ),
            (
                "conditional_manager_break_control.py",
                include_str!(
                    "fixtures/borrow_finally_reachability/conditional_manager_break_control.py"
                ),
            ),
            (
                "manager_following_work_control.py",
                include_str!(
                    "fixtures/borrow_finally_reachability/manager_following_work_control.py"
                ),
            ),
            (
                "manager_small_return_control.py",
                include_str!(
                    "fixtures/borrow_finally_reachability/manager_small_return_control.py"
                ),
            ),
        ] {
            assert_matches_cpython_marshal(source, name);
        }
    }

    #[test]
    fn matches_cpython_marshal_for_generator_manager_recovery_owners() {
        for (name, source) in [
            (
                "yield_manager_recovery_target.py",
                include_str!(
                    "fixtures/borrow_finally_reachability/yield_manager_recovery_target.py"
                ),
            ),
            (
                "yield_no_handler_control.py",
                include_str!("fixtures/borrow_finally_reachability/yield_no_handler_control.py"),
            ),
        ] {
            assert_matches_cpython_marshal(source, name);
        }
    }

    #[test]
    fn executes_control_flow() {
        let source = r#"
x = 3
total = 0
while x:
    total += x
    x -= 1

if total == 6:
    print("ok")
else:
    print("bad")
"#;
        assert_executes(source, "ok\n");
    }

    #[test]
    fn executes_simple_functions() {
        let source = r"
def add(left, right):
    result = left + right
    return result

print(add(20, 22))
";
        assert_executes(source, "42\n");
    }

    #[test]
    fn executes_literals_and_collections() {
        let source = r#"
values = [1, 2, 3]
pair = ("answer", 42)
mapping = {"answer": 42}
print(values[1], pair[0], mapping["answer"])
"#;
        assert_executes(source, "2 answer 42\n");
    }

    #[test]
    fn executes_short_circuit_expressions() {
        let source = r"
zero = 0
one = 1
print(zero and missing, one or missing, 1 if zero else 2, not zero)
";
        assert_executes(source, "0 1 2 True\n");
    }

    #[test]
    fn executes_break_continue_and_while_else() {
        let source = r"
x = 4
total = 0
while x:
    x -= 1
    if x == 2:
        continue
    if x == 0:
        break
    total += x
else:
    total = 999
print(total)
";
        assert_executes(source, "4\n");
    }

    #[test]
    fn executes_function_globals_and_docstrings() {
        let source = r#"
base = 40
def answer(offset):
    "answer docs"
    return base + offset

print(answer(2), answer.__doc__)
"#;
        assert_executes(source, "42 answer docs\n");
    }

    #[test]
    fn executes_extended_arguments_and_large_integers() {
        let mut source = String::from("flag = False\nif flag:\n");
        for index in 0..300 {
            writeln!(&mut source, "    value_{index} = {index}").unwrap();
        }
        source.push_str("else:\n    value_299 = 9223372036854775808\nprint(value_299)\n");

        assert_executes(&source, "9223372036854775808\n");
    }

    #[test]
    fn executes_for_loops_imports_and_globals() {
        let source = r"
import math as mathematics
from operator import add as plus

total = 0
for value in [1, 2, 3]:
    if value == 2:
        continue
    total = plus(total, value)
else:
    total += int(mathematics.sqrt(4))

def bump():
    global total
    total += 1

bump()
print(total)
";
        assert_executes(source, "7\n");
    }

    #[test]
    fn executes_unpacking_stores_and_deletes() {
        let source = r"
from types import SimpleNamespace

box = SimpleNamespace(value=1)
box.value += 4
values = [10, 20, 30]
values[1] += 2
head, *middle, tail = values
box.extra = head + tail
del values[0]
del box.extra
print(box.value, values, head, middle, tail, hasattr(box, 'extra'))
";
        assert_executes(source, "5 [22, 30] 10 [22] 30 False\n");
    }

    #[test]
    fn executes_starred_calls_collections_slices_and_comparisons() {
        let source = r"
def combine(a, b, c):
    return a + b + c

args = (1, 2)
keywords = {'c': 3}
values = [0, *args, 3]
mapping = {'a': 1, **{'b': 2}}
result = combine(*args, **keywords)
print(result, values[1:3], mapping, 0 < result < 10, (captured := result))
";
        assert_executes(source, "6 [1, 2] {'a': 1, 'b': 2} True 6\n");
    }

    #[test]
    fn executes_full_function_signatures_and_decorators() {
        let source = r"
def identity(function):
    return function

@identity
def combine(a, b=2, *args, c=3, **kwargs):
    return a + b + sum(args) + c + kwargs['d']

print(combine(1, 4, 5, 6, c=7, d=8))
";
        assert_executes(source, "31\n");
    }

    #[test]
    fn executes_lambdas_with_full_signatures() {
        let source = r"
combine = lambda a, b=2, *args, c=3, **kwargs: a + b + sum(args) + c + kwargs['d']
print(combine(1, 4, 5, 6, c=7, d=8))
";
        assert_executes(source, "31\n");
    }

    #[test]
    fn executes_nested_closures() {
        let source = r"
def outer(x):
    y = 2
    def middle(z):
        def inner():
            return x + y + z
        return inner
    return middle

print(outer(1)(3)())
";
        assert_executes(source, "6\n");
    }

    #[test]
    fn executes_nonlocal_assignments() {
        let source = r"
def counter():
    value = 0
    def bump():
        nonlocal value
        value += 1
        return value
    return bump

bump = counter()
print(bump(), bump())
";
        assert_executes(source, "1 2\n");
    }

    #[test]
    fn executes_fstrings_and_arbitrary_precision_integers() {
        let source = r#"
value = 1234567890123456789012345678901234567890
width = 6
print(f"value={value!r} suffix={value:0{width}d}")
print(f"{width=}")
"#;
        assert_executes(
            source,
            "value=1234567890123456789012345678901234567890 suffix=1234567890123456789012345678901234567890\nwidth=6\n",
        );
    }

    #[test]
    fn executes_classes_bases_decorators_and_methods() {
        let source = r"
def identity(value):
    return value

class Base:
    base = 40

@identity
class Answer(Base):
    offset = 2
    def value(self):
        return self.base + self.offset

print(Answer().value(), Answer.__firstlineno__, Answer.__static_attributes__)
";
        assert_executes(source, "42 8 ()\n");
    }

    #[test]
    fn executes_deferred_function_annotations() {
        let source = r"
def concrete(value: int) -> str:
    return str(value)

print(concrete.__annotations__)
";
        assert_executes(
            source,
            "{'value': <class 'int'>, 'return': <class 'str'>}\n",
        );

        let source = r"
from __future__ import annotations

def deferred(value: list[int]) -> str:
    return str(value)

print(deferred.__annotations__)
";
        assert_executes(source, "{'value': 'list[int]', 'return': 'str'}\n");
    }

    #[test]
    fn executes_future_module_annotations() {
        let source = r"
from __future__ import annotations

answer: int = 42
values: list[str]
print(__annotations__)
";
        assert_executes(source, "{'answer': 'int', 'values': 'list[str]'}\n");
    }

    #[test]
    fn executes_deferred_module_annotations() {
        let source = r"
answer: int = 42
values: list[str]
print(__annotate__(1))
";
        assert_executes(source, "{'answer': <class 'int'>, 'values': list[str]}\n");
    }

    #[test]
    fn executes_try_except_else_and_named_handlers() {
        let source = r#"
def parse(flag):
    try:
        if flag:
            raise ValueError("bad")
        value = 40
    except ValueError as error:
        value = len(str(error))
    else:
        value += 2
    return value

print(parse(False), parse(True))
"#;
        assert_executes(source, "42 3\n");
    }

    #[test]
    fn executes_try_finally_on_normal_and_exceptional_paths() {
        let source = include_str!("fixtures/execution/try_finally.py");
        assert_executes(
            source,
            "['normal', 'cleanup-1', 'handled', 'cleanup-2']\n2 ['normal', 'cleanup-1', 'handled', 'cleanup-2', 'value', 'override']\nbroken [('try', 1), ('break', 1)]\n[('try', 1), ('continue', 1), ('try', 2), ('continue', 2)]\n",
        );
    }

    #[test]
    fn executes_context_managers_and_exception_suppression() {
        let source = fixtures::CONTEXT_MANAGERS;
        assert_executes(
            source,
            "['enter', 42, 'exit', 'enter', 'exit', 'enter', 'enter', 'exit', 'exit', 'enter', 0, 'exit', 'enter', 1, 'exit', 'handled-0', 'handled-1']\n",
        );
    }

    #[test]
    fn executes_inlined_comprehensions_without_leaking_targets() {
        let source = r"
x = 100
values = [x * 2 for x in [-1, 1, 2] if x > 0]
pairs = {(x, y) for x in [1, 2] for y in [3, 4] if y > 3}
mapping = {x: x * 2 for x in [1, 2]}
print(values, sorted(pairs), mapping, x)
";
        assert_executes(source, "[2, 4] [(1, 4), (2, 4)] {1: 2, 2: 4} 100\n");
    }

    #[test]
    fn executes_generators_and_generator_expressions() {
        let source = r"
def numbers():
    yield 1
    yield 2

def all_numbers():
    yield from numbers()

def capture():
    list((last := value) for value in numbers())
    return last

yielding_lambda = lambda: (yield 3)
delegating_lambda = lambda: (yield from numbers())

generated = (value * 2 for value in numbers() if value > 1)
print(list(all_numbers()), list(generated), capture(), list(yielding_lambda()), list(delegating_lambda()))
";
        assert_executes(source, "[1, 2] [4] 2 [3] [1, 2]\n");
    }

    #[test]
    fn executes_coroutines_and_async_generators() {
        let source = include_str!("fixtures/execution/coroutines.py");
        assert_executes(
            source,
            "(42, 1, 2) [1, 2, 'done'] [1] ['enter', 42, 'exit', 'enter', 'ValueError'] ([4], {1, 2}, {1: 2, 2: 4}, [1, 2]) [42, 42]\n43 ['enter', 'exit'] [42, 42] ['enter', 'exit', 'enter', 'exit']\n",
        );
    }

    #[test]
    fn executes_structural_pattern_matching() {
        let source = include_str!("fixtures/execution/pattern_matching.py");
        assert_executes(
            source,
            "none\n('number', 2)\n('mapping', 1, {'y': 2})\n('sequence', 1, [2, 3], 4)\n('point', 5)\nother\n",
        );
    }

    #[test]
    fn executes_template_strings() {
        let source = r"
name = 'world'
width = 8
template = t'hello {name!r:>{width}}!'
interpolation = template.interpolations[0]
print(template.strings)
print(interpolation.value, interpolation.expression, interpolation.conversion, interpolation.format_spec)
";
        assert_executes(source, "('hello ', '!')\nworld name r >8\n");
    }

    #[test]
    fn executes_starred_annotations_and_class_arguments() {
        let source = r"
bases = (object,)
options = {}

class Dynamic(*bases, **options):
    marker = 42

def annotated(*args: *tuple[int]):
    pass

print(Dynamic.marker, annotated.__annotate__(1))
";
        assert_executes(source, "42 {'args': *tuple[int]}\n");
    }

    #[test]
    fn executes_type_aliases_and_generic_functions() {
        let source = r"
type Plain = list[int]
type Pair[T] = tuple[T, T]

def identity[T](value: T) -> T:
    return value

def bounded[T: int = bool](value: T) -> T:
    return value

class Box[T]:
    def __init__(self, value):
        self.value = value

print(Plain.__value__, Plain.__type_params__)
print(Pair.__type_params__, Pair[int], Pair[int].__value__)
print(identity(42), identity.__type_params__, identity.__annotate__(1))
print(bounded.__type_params__[0].__bound__, bounded.__type_params__[0].__default__)
print(Box.__type_params__, Box[int], Box[int](42).value)
";
        assert_executes(
            source,
            "list[int] ()\n(T,) Pair[int] tuple[T, T]\n42 (T,) {'value': T, 'return': T}\n<class 'int'> <class 'bool'>\n(T,) __main__.Box[int] 42\n",
        );
    }

    #[test]
    fn matches_cpython_marshal_for_tail_finally_cleanup_condition_in_with() {
        let source = r#"def f(cm, cw, cond):
    with cm() as tmp:
        with cw():
            for i in range(1, 3):
                try:
                    body(tmp)
                except Exception as e:
                    msg = f"failed: {e}"
                    raise AssertionError(msg)
                finally:
                    if cond:
                        cleanup()
"#;
        assert_matches_cpython_marshal(source, "tail_finally_cleanup_condition_in_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_followed_finally_cleanup_condition_in_with() {
        let source = r#"def f(cm, cond, max_workers):
    with cm() as manager:
        try:
            futures = []
            for arg in all_args:
                futures.append(manager.submit(*arg))
        except RuntimeError as e:
            import pytest
            pytest.skip(f"Spawning failed with error {e!r}")
        finally:
            if len(futures) < max_workers and cond:
                cleanup()
        for future in futures:
            future.result()
"#;
        assert_matches_cpython_marshal(source, "followed_finally_cleanup_condition_in_with.py");
    }

    #[test]
    fn matches_cpython_marshal_for_linear_assert_conditions_and_fallbacks() {
        let source = r"def comparisons(value, other, items):
    assert value == other
    assert value < other
    assert value in items
    assert value is other
    assert value is not other
    assert value.method() != other.method()
    assert (
        value.attribute
        == other.attribute
    )

def truth_tests(value, items, predicate):
    assert value
    assert value.attribute
    assert items[0]
    assert predicate(value)

def fallbacks(value, other, predicate):
    assert value is None
    assert value is not None
    assert not value
    assert value and other
    assert value or other
    assert value if predicate() else other
    assert value < other < predicate()
    assert (value, other)
    assert __debug__

def protected(manager, values, predicate):
    for value in values:
        assert predicate(value)
    with manager:
        assert predicate(values)
    try:
        assert predicate(values)
    except Exception:
        assert predicate(values), 'message'

def generator(values, predicate):
    yield values
    assert predicate(values)
";
        assert_matches_cpython_marshal(source, "linear_assert_conditions_and_fallbacks.py");
    }

    #[test]
    fn matches_cpython_marshal_for_normal_async_for_pop_lineage() {
        let source = include_str!("fixtures/normal_async_for_pop_lineage.py");
        assert_matches_cpython_marshal(source, "normal_async_for_pop_lineage.py");
    }

    #[test]
    fn matches_cpython_marshal_for_folded_loop_with_pop_location() {
        let source = include_str!("fixtures/folded_loop_with_pop_location.py");
        assert_matches_cpython_marshal(source, "folded_loop_with_pop_location.py");
    }
}

#[path = "compatibility_cases/handler_loop_continuation_locations.rs"]
mod handler_loop_continuation_locations;

#[path = "compatibility_cases/optimized_generator_join_owners.rs"]
mod optimized_generator_join_owners;

#[path = "compatibility_cases/optimized_boolean_backedge_owners.rs"]
mod optimized_boolean_backedge_owners;

#[path = "compatibility_cases/async_for_exhaustion_owners.rs"]
mod async_for_exhaustion_owners;

#[path = "compatibility_cases/terminal_and_while_tail.rs"]
mod terminal_and_while_tail;

#[path = "compatibility_cases/sourceless_try_body_exit.rs"]
mod sourceless_try_body_exit;

#[path = "compatibility_cases/copied_finally_continue_tail.rs"]
mod copied_finally_continue_tail;

#[path = "compatibility_cases/sourceless_try_else_exit.rs"]
mod sourceless_try_else_exit;

#[path = "compatibility_cases/for_body_owner_slots.rs"]
mod for_body_owner_slots;

#[path = "compatibility_cases/handler_continue_cleanup_owners.rs"]
mod handler_continue_cleanup_owners;

#[path = "compatibility_cases/shared_original_jump_aliases.rs"]
mod shared_original_jump_aliases;

#[path = "compatibility_cases/nested_async_terminal_conditionals.rs"]
mod nested_async_terminal_conditionals;

#[path = "compatibility_cases/original_empty_loop_backedge.rs"]
mod original_empty_loop_backedge;

#[path = "compatibility_cases/original_named_cleanup_join.rs"]
mod original_named_cleanup_join;

#[test]
fn matches_cpython_name_returns_after_terminal_handlers_with_awaited_finally() {
    for (source, filename) in [
        (
            r"async def run(channel, result):
    try:
        try:
            await channel.work()
        except Error as error:
            raise convert(error) from error
        return result
    finally:
        await channel.close()
",
            "awaited_finally_name_return.py",
        ),
        (
            r"async def run(channel, result):
    try:
        try:
            await channel.work()
        except Error as error:
            raise convert(error) from error
        return result
    finally:
        channel.close()
",
            "synchronous_call_finally_name_return_control.py",
        ),
        (
            r"async def run(channel, result):
    try:
        try:
            await channel.work()
        except Error as error:
            raise convert(error) from error
        return result
    finally:
        pass
",
            "pass_finally_name_return_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_midbody_continue_recovery_borrow_exits() {
    for (source, filename) in [
        (
            r#"def parse(source, decoder):
    parsed_values = []
    try:
        parsed_values.append(decode(source))
    except DecodeError:
        index = 0
        while True:
            index = source.find("{", index)
            if index == -1:
                break
            try:
                parsed, end = decoder.read(source, index)
            except DecodeError:
                index += 1
                continue
            parsed_values.append(parsed)
            index = end
    for parsed in parsed_values:
        result = convert(parsed)
        if result is not None:
            return result
    return None
"#,
            "continue_recovery_while_target.py",
        ),
        (
            r"def collect(options, value):
    result = {}
    try:
        batches = initial(value)
        for batch in batches:
            merge(result, batch)
    except Error:
        for option in options:
            try:
                batches = retry(option, value)
            except Error:
                continue
            for batch in batches:
                merge(result, batch)
    return [result[name] for name in sorted(result)]
",
            "continue_recovery_for_target.py",
        ),
        (
            r"def collect(options, value):
    result = {}
    try:
        batches = initial(value)
        for batch in batches:
            merge(result, batch)
    except Error:
        for option in options:
            try:
                batches = retry(option, value)
            except Error:
                batches = ()
            for batch in batches:
                merge(result, batch)
    return [result[name] for name in sorted(result)]
",
            "continue_recovery_fallthrough_control.py",
        ),
        (
            r"def collect(options, value):
    result = {}
    try:
        batches = initial(value)
        for batch in batches:
            merge(result, batch)
    except Error:
        for option in options:
            break
            try:
                batches = retry(option, value)
            except Error:
                continue
            for batch in batches:
                merge(result, batch)
    return [result[name] for name in sorted(result)]
",
            "continue_recovery_dead_prefix_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_folded_or_true_discarded_call_exits() {
    for (source, filename) in [
        (
            r"def run(exists, write, read, cache, items, consume, force):
    if not exists(cache) or True:
        values = []
        for item in items:
            values.append(item)
        write([(item,) for item in values], cache)
    else:
        values = [item for (item,) in read(cache)]
    result = consume(values)
    consume(cache, result, values)
",
            "folded_or_true_comprehension_call_tail.py",
        ),
        (
            r"def run(exists, write, read, cache, items, consume, force):
    if not exists(cache) or force:
        values = []
        for item in items:
            values.append(item)
        write([(item,) for item in values], cache)
    else:
        values = [item for (item,) in read(cache)]
    result = consume(values)
    consume(cache, result, values)
",
            "runtime_or_comprehension_call_tail_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_name_returns_with_conditional_finalizers() {
    for (source, filename) in [
        (
            r"def mutate(client, resource, credential, attempts):
    try:
        for attempt in range(attempts):
            current = read(resource)
            updated = normalize(current)
            try:
                write(resource, updated)
            except (Conflict, AlreadyExists) as error:
                last_error = error
                continue
            return updated
        raise Failure from last_error
    finally:
        if resource is not None:
            resource.close()
",
            "simple_existing_cleanup_control.py",
        ),
        (
            r"def mutate(client, resource, credential, attempts):
    try:
        for attempt in range(attempts):
            current = read(resource)
            updated = normalize(current)
            try:
                write(resource, updated)
            except (Conflict, AlreadyExists) as error:
                last_error = error
                continue
            return updated
        raise Failure from last_error
    finally:
        if resource is not None:
            resource.close()
            if credential is not None:
                credential.close()
",
            "simple_test_nested_cleanup.py",
        ),
        (
            r"def mutate(client, resource, credential, attempts):
    try:
        for attempt in range(attempts):
            current = read(resource)
            updated = normalize(current)
            try:
                write(resource, updated)
            except (Conflict, AlreadyExists) as error:
                last_error = error
                continue
            return updated
        raise Failure from last_error
    finally:
        if False:
            resource.close()
            if credential is not None:
                credential.close()
",
            "folded_false_cleanup_control.py",
        ),
        (
            r"def mutate(client, resource, credential, attempts):
    try:
        for attempt in range(attempts):
            current = read(resource)
            updated = normalize(current)
            try:
                write(resource, updated)
            except (Conflict, AlreadyExists) as error:
                last_error = error
                continue
            return updated
        raise Failure from last_error
    finally:
        if client is None and resource is not None:
            resource.close()
            if credential is not None:
                credential.close()
",
            "compound_cleanup.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_handler_retry_break_with_finalizer() {
    let (source, filename) = (
        r"def run(items, manager, value):
    try:
        work(value)
    except OuterError:
        error = None
        for item in items:
            prepare(item)
            try:
                retry(item, value)
                error = None
                break
            except InnerError as caught:
                error = caught
            finally:
                manager.finished(item)
        if error is not None:
            raise error
    consume(value)
",
        "retry_finalizer_break_target.py",
    );
    common::assert_matches_cpython_marshal(source, filename);
}

#[test]
fn matches_cpython_threaded_async_loop_exit_borrow_boundaries() {
    for (source, filename) in [
        (
            r"async def run(manager, source, sink):
    async with manager:
        try:
            try:
                async for item in await source():
                    break
            except Error as error:
                if bad(error) or worse(error):
                    raise Wrapped(error) from error
                record(error)
            try:
                await sink(source)
            except Error:
                pass
        except Error:
            recover(source)
    consume(source)
",
            "protected_async_direct_break.py",
        ),
        (
            r"async def run(manager, trace, source, value):
    async with manager:
        with trace:
            matched = False
            try:
                async for item in source:
                    if selected(item) and item.key == value:
                        matched = True
                        break
            except (Error, OtherError):
                pass
            if not matched:
                consume(value)
            consume(source)
",
            "protected_async_terminal_conditional_break.py",
        ),
        (
            r"async def run(manager, source, value):
    async with manager:
        try:
            async for item in source:
                if selected(item):
                    break
                consume(item)
        except Error:
            pass
        consume(value)
        consume(source)
",
            "protected_async_surviving_break_control.py",
        ),
        (
            r"async def run(manager, source, value):
    async with manager:
        try:
            async for item in source:
                break
        except Error:
            pass
        return value
",
            "protected_async_small_return_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_located_conditional_finally_cleanup_owners() {
    for (source, filename) in [
        (
            r"async def test(cm, proc):
    with cm:
        try:
            await work()
        finally:
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.communicate(timeout=5)
                except TimeoutError:
                    proc.kill()
                    proc.communicate(timeout=5)
",
            "01_async_outer_with_guarded_try.py",
        ),
        (
            r"async def test(cm, timeout, client):
    with cm:
        try:
            await work()
        except Exception as exc:
            recover(exc)
        finally:
            if client is not None:
                try:
                    async with timeout:
                        await client.cleanup()
                except Exception as exc:
                    recover(exc)
",
            "05_async_guarded_cleanup_async_with_try.py",
        ),
        (
            r"async def test(cm, suppressor, task):
    with cm:
        try:
            await work()
        except Exception:
            recover()
            raise
        finally:
            if task is not None:
                task.cancel()
                with suppressor:
                    await task
    done()
",
            "06_async_guarded_with_await_cleanup.py",
        ),
        (
            r"async def test(cm, second, lock, enabled):
    with cm, second:
        try:
            await work()
        except Exception as exc:
            recover(exc)
        finally:
            flush()
            if enabled:
                with lock:
                    collect()
",
            "07_async_two_outer_with_cleanup.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_located_conditional_finally_owner_boundaries() {
    for (source, filename) in [
        (
            r"async def test(proc):
    try:
        await work()
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.communicate(timeout=5)
            except TimeoutError:
                proc.kill()
                proc.communicate(timeout=5)
",
            "03_async_without_outer_with_control.py",
        ),
        (
            r"async def test(cm, enabled):
    with cm:
        try:
            await work()
        finally:
            if enabled:
                cleanup()
",
            "08_async_linear_conditional_cleanup_control.py",
        ),
        (
            r"async def test(cm, lock, enabled):
    async with cm:
        try:
            await work()
        finally:
            if enabled:
                with lock:
                    collect()
",
            "09_async_outer_async_with_control.py",
        ),
        (
            r"def test(cm, lock, enabled):
    with cm:
        try:
            yield work()
        finally:
            if enabled:
                with lock:
                    collect()
",
            "14_generator_guarded_cleanup_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_exceptional_finally_inner_iterator_returns() {
    for value in [
        "item",
        "convert(item)",
        r#"f"item {item}""#,
        "42",
        "None",
        "",
    ] {
        let source = format!(
            "def run(source, inner, enabled):\n    for outer in source:\n        try:\n            return outer\n        finally:\n            if enabled:\n                for item in inner:\n                    return {value}\n"
        );
        common::assert_matches_cpython_marshal(&source, "exceptional_finally_inner_iterator.py");
    }
}

#[test]
fn matches_cpython_exceptional_finally_iterator_depth_boundaries() {
    for (source, filename) in [
        (
            r"def run(source, enabled, pending):
    try:
        return pending
    finally:
        if enabled:
            for item in source:
                return item
",
            "exceptional_finally_no_outer.py",
        ),
        (
            r"def run(source, inner, enabled):
    for outer in source:
        try:
            return outer
        finally:
            if enabled:
                for group in inner:
                    for item in group:
                        return item
",
            "exceptional_finally_two_inner.py",
        ),
        (
            r"def run(source, inner, enabled):
    for outer in source:
        try:
            return outer
        finally:
            while enabled:
                for item in inner:
                    return item
                break
",
            "exceptional_finally_while_around_inner.py",
        ),
        (
            r"def run(source, enabled):
    for outer in source:
        try:
            return outer
        finally:
            while enabled:
                return outer
",
            "exceptional_finally_while_only.py",
        ),
        (
            r"def run(source, enabled):
    for outer in source:
        try:
            return outer
        finally:
            if enabled:
                return convert(outer)
",
            "exceptional_finally_outer_only.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

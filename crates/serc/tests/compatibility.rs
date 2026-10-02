mod common;
mod compatibility_cases;
#[path = "common/startup_reference.rs"]
mod startup_reference;

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
    fn exceptional_finally_break_copies_nested_handler_cleanup() {
        let source = r#"
def run(log):
    for i in range(1):
        try:
            try:
                raise RuntimeError("pending")
            finally:
                try:
                    break
                finally:
                    try:
                        raise KeyError("inner")
                    except KeyError:
                        log.append("handled")
        except Exception as error:
            log.append(("outer", type(error).__name__))
    return log
"#;
        assert_matches_cpython_marshal(source, "nested_handler_finally_break.py");
        let named = source.replace("except KeyError:", "except KeyError as error:");
        assert_matches_cpython_marshal(&named, "named_handler_finally_break.py");
        let traced = format!(
            r#"{source}
import sys
events = []
def trace(frame, event, arg):
    if frame.f_code.co_name == "run":
        frame.f_trace_opcodes = True
        if event == "opcode":
            events.append(frame.f_lasti)
    return trace
sys.settrace(trace)
print(run([]))
sys.settrace(None)
print(len(events))
"#
        );
        assert_executes(&traced, "['handled']\n37\n");
    }

    #[test]
    fn nested_constant_condition_matches_cpython_marshal() {
        for condition in [
            "False if d else False",
            "False if d else True",
            "0 if d else 0",
            "None if d else None",
            "False if d else 0",
            "0 if d else False",
            "False if d else None",
            "None if d else False",
        ] {
            let source = format!("def f(d):\n    return 1 if ({condition}) else 2\n");
            assert_matches_cpython_marshal(&source, "nested_constant_condition.py");
        }
        assert_executes(
            "class Probe:\n    def __bool__(self):\n        print('tested')\n        return True\n\ndef f(d):\n    return 1 if (False if d else False) else 2\n\nprint(f(Probe()))\n",
            "tested\n2\n",
        );
    }

    #[test]
    fn terminating_finally_keeps_pending_return_until_cleanup_finishes() {
        let source = r#"
class Tracked:
    def __del__(self):
        print("destroyed")

def pending():
    return Tracked()

def cleanup(raises):
    print("cleanup")
    if raises:
        raise ValueError()

def replace_return(raises):
    try:
        return pending()
    finally:
        cleanup(raises)
        return 42

print(replace_return(False))
try:
    replace_return(True)
except ValueError:
    print("caught")
"#;
        assert_matches_cpython_marshal(source, "terminating_finally_return.py");
        assert_executes(
            source,
            "cleanup\ndestroyed\n42\ncleanup\ndestroyed\ncaught\n",
        );
    }

    #[test]
    fn terminating_finally_keeps_pending_return_through_branches_and_assignments() {
        let source = r#"
class Tracked:
    def __del__(self):
        print("destroyed")

def pending():
    return Tracked()

def cleanup(raises):
    print("cleanup")
    if raises:
        raise ValueError()

def guarded(flag, raises):
    try:
        return pending()
    finally:
        flag and cleanup(raises)
        return 42

def folded(raises):
    try:
        return pending()
    finally:
        True and cleanup(raises)
        return 42

def conditional(flag, raises):
    try:
        return pending()
    finally:
        if flag:
            cleanup(raises)
        else:
            print("skipped")
        return 42

def assigned(raises):
    try:
        return pending()
    finally:
        result = cleanup(raises)
        return 42

print(guarded(False, False))
print(guarded(True, False))
try:
    guarded(True, True)
except ValueError:
    print("caught")
print(folded(False))
try:
    folded(True)
except ValueError:
    print("caught")
print(conditional(False, False))
print(conditional(True, False))
try:
    conditional(True, True)
except ValueError:
    print("caught")
print(assigned(False))
try:
    assigned(True)
except ValueError:
    print("caught")
"#;
        assert_matches_cpython_marshal(source, "branching_finally_return.py");
        assert_executes(
            source,
            "destroyed\n42\ncleanup\ndestroyed\n42\ncleanup\ndestroyed\ncaught\n\
             cleanup\ndestroyed\n42\ncleanup\ndestroyed\ncaught\n\
             skipped\ndestroyed\n42\ncleanup\ndestroyed\n42\ncleanup\ndestroyed\ncaught\n\
             cleanup\ndestroyed\n42\ncleanup\ndestroyed\ncaught\n",
        );
    }

    #[test]
    fn pending_returns_survive_manager_cleanup_and_suspension() {
        let source = r#"
events = []
class Held:
    count = 0
    def __init__(self):
        Held.count += 1
        events.append("create")
    def __del__(self):
        Held.count -= 1
        events.append("destroy")

class Guard:
    def __enter__(self):
        events.append(f"enter:{Held.count}")
    def __exit__(self, *args):
        events.append(f"exit:{Held.count}")

class Suspension:
    def __await__(self):
        yield None

def generator(flag):
    try:
        if flag:
            return Held()
        return Held()
    finally:
        with Guard():
            yield None
        return 42

async def coroutine(flag):
    try:
        if flag:
            return Held()
        return Held()
    finally:
        with Guard():
            await Suspension()
        return 42

for factory in (generator, coroutine):
    for flag in (False, True):
        for raises in (False, True):
            events.clear()
            pending = factory(flag)
            pending.send(None)
            print("live", Held.count)
            try:
                if raises:
                    pending.throw(ValueError())
                else:
                    pending.send(None)
            except StopIteration as stop:
                print("returned", stop.value)
            except ValueError:
                print("raised")
            print(events)
"#;
        assert_matches_cpython_marshal(source, "suspending_finally_return.py");
        assert_executes(
            source,
            &"live 1\nreturned 42\n['create', 'enter:1', 'exit:1', 'destroy']\n\
              live 1\nraised\n['create', 'enter:1', 'exit:1', 'destroy']\n"
                .repeat(4),
        );
    }

    #[test]
    fn pending_returns_survive_handler_and_else_finalizers() {
        let source = r#"
class Held:
    count = 0
    def __init__(self):
        Held.count += 1
        print("create")
    def __del__(self):
        Held.count -= 1
        print("destroy")

def replace(path):
    try:
        if path == 0:
            return Held()
        if path == 1:
            raise ValueError()
    except ValueError as error:
        return Held()
    else:
        return Held()
    finally:
        print("live", Held.count)
        return 42

def replace_group():
    try:
        return Held()
    except* ValueError:
        pass
    finally:
        print("live", Held.count)
        return 42

for path in range(3):
    print(replace(path))
print(replace_group())
"#;
        assert_matches_cpython_marshal(source, "handler_finally_return.py");
        assert_executes(source, &"create\nlive 1\ndestroy\n42\n".repeat(4));
    }

    #[test]
    fn folded_percent_format_cleanup_preserves_discard_positions() {
        for expression in [
            "True and ('%s' % (value,))",
            "False or ('%s' % (value,))",
            "True and ('%r' % (value,))",
            "True and ('x:%s' % (value,))",
        ] {
            let source = format!(
                "def f(value, pending):\n    try:\n        return pending()\n    finally:\n        {expression}\n        return 42\n"
            );
            assert_matches_cpython_marshal(&source, "folded_percent_finally.py");
        }
    }

    #[test]
    fn exposes_the_pinned_cpython_source_identity() {
        assert_eq!(CPYTHON_TARGET.tag, "v3.14.5");
        assert_eq!(
            CPYTHON_TARGET.commit,
            "5607950ef232dad16d75c0cf53101d9649d89115"
        );
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
            CompileOptions, InitialInternState, compile_bytes_with_options,
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
        for python_version in crate::common::cpython::VERSIONS {
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
            CompileOptions, InitialInternState, SourceDecodeError, compile_with_intern_state,
            compile_with_options,
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
            for python_version in crate::common::cpython::VERSIONS {
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
            CompileOptions, InitialInternState, SourceDecodeError, compile_bytes_with_intern_state,
            compile_bytes_with_options, compile_with_intern_state, compile_with_options,
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
            for python_version in crate::common::cpython::VERSIONS {
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
            CompileOptions, SourceDecodeError, compile_bytes_with_options,
            compile_bytes_with_path_and_options,
        };

        let sources: &[&[u8]] = &[b"pass", b"if =", b"\0", b"\xff", b"# coding: unknown\npass"];
        for python_version in crate::common::cpython::VERSIONS {
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
        use std::os::unix::ffi::OsStrExt as _;
        use std::path::Path;

        use serc::{CompileOptions, compile_bytes_with_path_and_options};

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

        for python_version in crate::common::cpython::VERSIONS {
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
                let expected = crate::common::cpython::run_with_input(
                    crate::common::cpython::command(python_version)
                        .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle_script])
                        .args([
                            &hex,
                            target.tag.strip_prefix('v').unwrap(),
                            &target.magic_hex(),
                        ]),
                    source,
                );
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

    const SOURCE_CODEC_IDENTITY_BODY: &str = "def first(value):\n    return value in {'encoding_table', 'novel_marker'}, 1\n\ndef second(value):\n    'second docstring'\n    return value in {'encoding_table', 'novel_marker'}, 2\n";

    #[test]
    fn matches_cpython_marshal_for_utf8_codec_identity_control() {
        let source = format!("# coding: utf-8\n{SOURCE_CODEC_IDENTITY_BODY}");
        assert_matches_cpython_marshal_bytes(source.as_bytes(), "source_codec_identity.py");
    }

    #[test]
    fn matches_cpython_marshal_when_source_codecs_intern_constants() {
        for encoding in [
            "cp1251",
            "windows.1251",
            "iso-8859-15",
            "latin9",
            "koi8-r",
            "cskoi8r",
        ] {
            let source = format!("# coding: {encoding}\n{SOURCE_CODEC_IDENTITY_BODY}");
            assert_matches_cpython_marshal_bytes(source.as_bytes(), "source_codec_identity.py");
        }
    }

    #[test]
    fn matches_codec_imports_and_registry_alias_interning() {
        for (encoding, value) in [
            ("cp1251", "decoding_table"),
            ("euc-jp", "__map_jisx0208"),
            ("euc-jp", "__create_codec"),
            ("euc-jp", "readlines"),
            ("euc-jp", "writelines"),
            ("euc-jp", "stream"),
            ("cp367", "ascii_encode"),
            ("cp819", "latin_1_encode"),
            ("cp65001", "cp65001"),
            ("u.jis", "u.jis"),
        ] {
            let body = format!(
                "def first(value):\n    return value in {{{value:?}, 'novel_marker'}}, 1\n\ndef second(value):\n    'second docstring'\n    return value in {{{value:?}, 'novel_marker'}}, 2\n"
            );
            let source = format!("# coding: {encoding}\n{body}");
            assert_matches_cpython_marshal_bytes(source.as_bytes(), "codec_effects.py");
        }
    }

    #[test]
    fn matches_source_codec_interns_for_all_targets() {
        use std::path::Path;

        use serc::{
            CompileOptions, compile_bytes_with_options, compile_bytes_with_path_and_options,
        };

        let script = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.buffer.read(), 'codec_effects.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
        for version in crate::common::cpython::VERSIONS {
            let target = version.target();
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            for (encoding, value) in [
                ("utf-8", "encoding_table"),
                ("cp1251", "encoding_table"),
                ("cp1251", "Codec.encode"),
                ("iso-8859-15", "decoding_table"),
                ("koi8-r", "encoding_table"),
                ("euc-jp", "__map_jisx0208"),
                ("euc-jp", "readlines"),
                ("euc-jp", "writelines"),
                ("euc-jp", "stream"),
                ("cp367", "ascii_encode"),
                ("cp819", "latin_1_encode"),
                ("cp65001", "cp65001"),
            ] {
                let source = if value == "Codec.encode" {
                    format!("# coding: {encoding}\nclass Codec:\n    class encode: pass\n")
                } else {
                    format!(
                        "# coding: {encoding}\ndef first(value):\n    return value in {{{value:?}, 'novel_marker'}}, 1\n\ndef second(value):\n    'second docstring'\n    return value in {{{value:?}, 'novel_marker'}}, 2\n"
                    )
                };
                let expected = crate::common::cpython::run_with_input(
                    crate::common::cpython::command(version)
                        .args(["-B", "-I", "-S", "-X", "utf8", "-c", script])
                        .args([target.tag.strip_prefix('v').unwrap(), &target.magic_hex()]),
                    source.as_bytes(),
                );
                assert!(
                    expected.status.success(),
                    "{version}, {encoding}: {}",
                    String::from_utf8_lossy(&expected.stderr)
                );
                let named =
                    compile_bytes_with_options(source.as_bytes(), "codec_effects.py", &options)
                        .unwrap()
                        .marshal();
                let path = compile_bytes_with_path_and_options(
                    source.as_bytes(),
                    Path::new("codec_effects.py"),
                    &options,
                )
                .unwrap()
                .marshal();
                assert_eq!(named, expected.stdout, "{version}, {encoding}");
                assert_eq!(path, expected.stdout, "{version}, {encoding}, path");
            }
        }
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
    fn python_312_rejects_starred_type_parameter_defaults() {
        use serc::{CompileOptions, PythonVersion, compile_with_options};

        let options = CompileOptions {
            python_version: PythonVersion::Py312,
            ..CompileOptions::default()
        };
        for (source, expected_message, expected_range) in [
            (
                "type Alias[*Ts = tuple[int]] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = tuple[int]",
            ),
            (
                "type Café[*Ts = (int)] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = (int",
            ),
            (
                "type Alias[T, *Ts = (int, str),] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = (int, str)",
            ),
            (
                "type Alias[T: int, *Ts = int] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = int",
            ),
            (
                "type Alias[*Ts = int, T = int] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = int",
            ),
            (
                "type Alias[*Ts # = ignored\n = int] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts # = ignored\n = int",
            ),
            (
                "if False:\n    type Alias[*Ts = factory()] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = factory()",
            ),
            (
                "value = 1; type Alias[*Ts = int] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = int",
            ),
            ("type Alias[*Ts = *tuple[int]] = int", "invalid syntax", "="),
            (
                "type Alias[*Ts # = ignored\n = *int] = int",
                "invalid syntax",
                "=",
            ),
            ("def f[*Ts = int](): pass", "expected '('", "["),
            ("class C[*Ts = int]: pass", "invalid syntax", "="),
            (
                "break\ntype Alias[*Ts = int] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = int",
            ),
        ] {
            let error = compile_with_options(source, "type_defaults.py", &options).unwrap_err();
            let CompileError::InvalidSyntax { message, range } = error else {
                panic!("expected invalid syntax for {source:?}, got {error:?}");
            };
            assert_eq!(message, expected_message, "{source:?}");
            assert_eq!(&source[range], expected_range, "{source:?}");
        }

        // A syntax error in an incomplete default must not be replaced by the later
        // invalid-unpacking diagnostic.
        for source in [
            "type Alias[*Ts = (1 + )] = int",
            "type Alias[*Ts = x + ] = int",
            "type Alias[*Ts = x if cond] = int",
            "if:\ntype Alias[*Ts = int] = int",
            "type Alias[*Ts = int] = int\nif:",
            "type Alias[*Ts = int] = int\nx = 'unterminated\n",
            "type Alias[*Ts = int] = int\nx = )",
            "type Alias[*Ts = int] = int\nx = 1__2",
            "type Alias[*Ts = int] = int\nx = 0123",
        ] {
            let error = compile_with_options(source, "type_defaults.py", &options).unwrap_err();
            assert!(
                matches!(error, CompileError::Parse(_)),
                "{source:?}: {error:?}"
            );
        }
        let source = "type Alias[T = int, *Ts = int] = int";
        let error = compile_with_options(source, "type_defaults.py", &options).unwrap_err();
        assert!(!error.to_string().contains("iterable argument unpacking"));
    }

    #[test]
    fn python_312_prefers_the_first_type_parameter_default_error() {
        use serc::{CompileOptions, PythonVersion, compile_with_options};

        let options = CompileOptions {
            python_version: PythonVersion::Py312,
            ..CompileOptions::default()
        };
        for (source, expected_message, expected_range) in [
            (
                "type A[*Ts = int, **P = ...] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = int",
            ),
            ("type A[**P = ..., *Ts = int] = int", "invalid syntax", "="),
            ("class A[*Ts = int, **P = ...]: pass", "invalid syntax", "="),
            ("class A[**P = ..., *Ts = int]: pass", "invalid syntax", "="),
            ("def f[*Ts = int, **P = ...](): pass", "expected '('", "["),
            ("def f[**P = ..., *Ts = int](): pass", "expected '('", "["),
            (
                "type A[**P = ...] = int\ntype B[*Ts = int] = int",
                "invalid syntax",
                "=",
            ),
            (
                "type A[*Ts = int] = int\ntype B[**P = ...] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = int",
            ),
            (
                "type A[**P = ...] = int; type B[*Ts = int] = int",
                "invalid syntax",
                "=",
            ),
            (
                "type A[*Ts = int] = int; type B[**P = ...] = int",
                "cannot assign to iterable argument unpacking",
                "*Ts = int",
            ),
            (
                "class A[**P = ...]: pass\ntype B[*Ts = int] = int",
                "invalid syntax",
                "=",
            ),
            (
                "def f[**P = ...](): pass\ntype B[*Ts = int] = int",
                "expected '('",
                "[",
            ),
            (
                "class A[*Ts = int]: pass\ntype B[**P = ...] = int",
                "invalid syntax",
                "=",
            ),
            (
                "if True:\n    type A[**P = ...] = int\n    type B[*Ts = int] = int",
                "invalid syntax",
                "=",
            ),
        ] {
            let error = compile_with_options(source, "type_defaults.py", &options).unwrap_err();
            let CompileError::InvalidSyntax { message, range } = error else {
                panic!("expected invalid syntax for {source:?}, got {error:?}");
            };
            assert_eq!(message, expected_message, "{source:?}");
            assert_eq!(&source[range], expected_range, "{source:?}");
            assert_eq!(
                usize::from(range.start()),
                source.find(expected_range).unwrap()
            );
        }

        for source in [
            "type A[**P = ...] = int\nif:",
            "if:\ntype A[**P = ...] = int\ntype B[*Ts = int] = int",
            "type A[**P = ...] = int\ntype B[*Ts = int] = int\nif:",
            "type A[*Ts = int] = int\ntype B[**P = ...] = int\nx = 'unterminated\n",
        ] {
            assert!(matches!(
                compile_with_options(source, "type_defaults.py", &options),
                Err(CompileError::Parse(_))
            ));
        }
    }

    #[test]
    fn starred_type_parameters_and_later_defaults_remain_supported() {
        use serc::{CompileOptions, PythonVersion, compile_with_options};

        let options = CompileOptions {
            python_version: PythonVersion::Py312,
            ..CompileOptions::default()
        };
        for source in ["type Alias[*Ts] = int", "type = 1; values = [*items]"] {
            compile_with_options(source, "type_defaults.py", &options).unwrap();
        }

        for python_version in [
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = CompileOptions {
                python_version,
                ..CompileOptions::default()
            };
            for source in [
                "type Alias[*Ts = tuple[int]] = int",
                "type Alias[*Ts = *tuple[int]] = int",
                "def f[*Ts = int](): pass",
                "class C[*Ts = int]: pass",
            ] {
                compile_with_options(source, "type_defaults.py", &options)
                    .unwrap_or_else(|error| panic!("{python_version:?}: {source:?}: {error}"));
            }
        }
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
    fn matches_cpython_marshal_when_a_match_case_ends_in_a_constant_expression() {
        let source = fixtures::MATCH_CONTROL_FLOW;
        assert_matches_cpython_marshal(source, "match.py");
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
    fn reports_the_original_duplicate_pattern_binding_name() {
        let source = "class Container:\n    match value:\n        case [__captured, [__captured] | [__captured]]:\n            pass\n";
        let error = compile(source, "duplicate_pattern.py").unwrap_err();

        assert_eq!(
            error.to_string(),
            "failed to parse Python source: multiple assignments to name '__captured' in pattern"
        );
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
    fn matches_cpython_marshal_for_generator_nested_comprehension_captures() {
        let source = include_str!("fixtures/generator_nested_comprehension_captures.py");
        assert_matches_cpython_marshal(source, "generator_nested_comprehension_captures.py");
        assert_executes(&format!("{source}\nprint(list(empty()))\n"), "[(42, [])]\n");
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
    fn preserves_enclosing_finally_owner_for_nested_pass_finally() {
        for (function, trailer) in [("def", ""), ("async def", ""), ("def", "    yield 1\n")] {
            for (body, handler) in [("work()", ""), ("pass", " Exception")] {
                let source = format!(
                    "{function} f():\n    try:\n        try:\n            {body}\n        finally:\n            pass\n    except{handler}:\n        recover()\n    finally:\n        cleanup()\n{trailer}"
                );
                assert_matches_cpython_marshal(&source, "nested_pass_finally.py");
                assert_matches_cpython_marshal_bytes(source.as_bytes(), "nested_pass_finally.py");
            }
        }
        // The same exit at a loop backedge retains its existing owner.
        let loop_source = "def f(items):\n    try:\n        for item in items:\n            try:\n                work()\n            finally:\n                pass\n    except:\n        recover()\n    finally:\n        cleanup()\n";
        assert_matches_cpython_marshal(loop_source, "loop_pass_finally.py");
    }

    #[test]
    fn trace_error_runs_outer_finally_for_nested_pass_finally() {
        let source = r"import sys

events = []
def f():
    try:
        try:
            events.append('work')
        finally:
            pass
    except:
        events.append('handler')
    finally:
        events.append('cleanup')

def trace(frame, event, arg):
    if frame.f_code is f.__code__ and event == 'line' and frame.f_lineno == f.__code__.co_firstlineno + 5:
        raise RuntimeError('trace failed')
    return trace

sys.settrace(trace)
try:
    f()
except RuntimeError:
    events.append('raised')
finally:
    sys.settrace(None)
print(events)
";
        assert_executes(source, "['work', 'cleanup', 'raised']\n");
    }

    #[test]
    fn generator_stop_iteration_owner_survives_nested_pass_finally() {
        for handler in [
            "except:\n        recover()",
            "except ValueError:\n        recover()",
            "except ValueError as error:\n        recover(error)",
            "except ValueError:\n        recover()\n    except TypeError:\n        recover()",
        ] {
            let source = format!(
                "def f():\n    try:\n        try:\n            work()\n        finally:\n            pass\n    {handler}\n    yield 1\n"
            );
            assert_matches_cpython_marshal(&source, "generator_pass_finally.py");
            assert_matches_cpython_marshal_bytes(source.as_bytes(), "generator_pass_finally.py");
        }
    }

    #[test]
    fn trace_stop_iteration_in_nested_pass_finally_is_converted() {
        let source = r"import sys

events = []
failing = False
injected = StopIteration

def work():
    events.append('work')
    if failing:
        raise ValueError('work failed')

def recover():
    events.append('recover')

def f():
    try:
        try:
            work()
        finally:
            pass
    except:
        recover()
    yield 1

def trace(frame, event, arg):
    if frame.f_code is f.__code__ and event == 'line' and frame.f_lineno == f.__code__.co_firstlineno + 5:
        events.append('trace')
        raise injected('trace failed')
    return trace

for failing in (False, True):
    for injected in (StopIteration, ValueError):
        events = []
        try:
            sys.settrace(trace)
            events.append(('yield', next(f())))
        except BaseException as error:
            events.append((type(error).__name__, type(error.__cause__).__name__, type(error.__context__).__name__))
        finally:
            sys.settrace(None)
        print(events)
";
        assert_executes(
            source,
            "['work', 'trace', ('RuntimeError', 'StopIteration', 'StopIteration')]\n['work', 'trace', ('ValueError', 'NoneType', 'NoneType')]\n['work', 'trace', 'recover', ('yield', 1)]\n['work', 'trace', 'recover', ('yield', 1)]\n",
        );
    }

    #[test]
    fn nonterminal_pass_finally_preserves_enclosing_handler() {
        for (function, trailer) in [
            ("def", ""),
            ("async def", ""),
            ("def", "    yield 1\n"),
            ("async def", "    yield 1\n"),
        ] {
            for handler in ["", " ValueError", " ValueError as error"] {
                for successor in [
                    "after()",
                    "pass\n        after()",
                    "return",
                    "raise ValueError",
                ] {
                    let source = format!(
                        "{function} f():\n    try:\n        try:\n            work()\n        finally:\n            pass\n        {successor}\n    except{handler}:\n        recover()\n{trailer}"
                    );
                    assert_matches_cpython_marshal(&source, "nonterminal_pass_finally.py");
                    assert_matches_cpython_marshal_bytes(
                        source.as_bytes(),
                        "nonterminal_pass_finally.py",
                    );
                }
            }
        }
    }

    #[test]
    fn trace_error_in_nonterminal_pass_finally_enters_enclosing_handler() {
        let source = r"import sys

events = []
failing = False
injected = RuntimeError

def work():
    events.append('work')
    if failing:
        raise ValueError('work failed')

def after():
    events.append('after')

def recover():
    events.append('recover')

def f():
    try:
        try:
            work()
        finally:
            pass
        after()
    except:
        recover()
    yield 1

def trace(frame, event, arg):
    if frame.f_code is f.__code__ and event == 'line' and frame.f_lineno == f.__code__.co_firstlineno + 5:
        events.append('trace')
        raise injected('trace failed')
    return trace

for failing in (False, True):
    for injected in (RuntimeError, StopIteration):
        events = []
        try:
            sys.settrace(trace)
            events.append(('yield', next(f())))
        finally:
            sys.settrace(None)
        print(events)
";
        assert_executes(
            source,
            &"['work', 'trace', 'recover', ('yield', 1)]\n".repeat(4),
        );
    }

    #[test]
    fn preserves_try_star_boolean_value_marker_owners() {
        let source = include_str!("fixtures/try_star_boolean_value_owners.py");
        assert_executes(source, "True\nTrue\n");
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
    fn matches_cpython_terminal_async_for_nop_ownership() {
        let source = "import dis\n\nasync def sync_with_direct(manager, items):\n    with manager:\n        async for item in items:\n            consume(item)\n\nasync def async_then_sync_with(async_manager, sync_manager, items):\n    async with async_manager:\n        with sync_manager:\n            async for item in items:\n                consume(item)\n\nasync def try_around_async_with(manager, items):\n    try:\n        async with manager:\n            async for item in items:\n                consume(item)\n    finally:\n        cleanup()\n\nasync def async_with_around_try(manager, items):\n    async with manager:\n        try:\n            async for item in items:\n                consume(item)\n        finally:\n            cleanup()\n\ndef has_end_nop(function):\n    instructions = list(dis.get_instructions(function))\n    return any(\n        left.opname == 'END_ASYNC_FOR' and right.opname == 'NOP'\n        for left, right in zip(instructions, instructions[1:])\n    )\n\nfor function in (sync_with_direct, async_then_sync_with, try_around_async_with, async_with_around_try):\n    print(has_end_nop(function))\n";
        assert_executes(source, "True\nTrue\nFalse\nTrue\n");
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
    fn executes_arithmetic_names_and_calls() {
        assert_executes("answer = 6 * 7\nprint(answer)\n", "42\n");
    }

    #[test]
    fn executes_copied_finally_handler_returns() {
        let source = include_str!("fixtures/execution/copied_finally_handler_returns.py");
        assert_executes(source, "ok\n");
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
}

#[path = "compatibility_cases/handler_loop_continuation_locations.rs"]
mod handler_loop_continuation_locations;

#[path = "compatibility_cases/optimized_generator_join_owners.rs"]
mod optimized_generator_join_owners;

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

#[path = "compatibility_cases/original_empty_loop_backedge.rs"]
mod original_empty_loop_backedge;

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
fn nested_finally_boolean_marker_does_not_catch_trace_exception() {
    let source = r"import dis
import sys

def consume(value):
    pass

def f(flag):
    for _ in range(2):
        try:
            try:
                consume('body')
            finally:
                try:
                    consume('finally')
                    break
                finally:
                    flag and consume(1)
        except Exception:
            consume('except')

def trace(frame, event, arg):
    if event == 'call' and frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        frame.f_trace_lines = False
        return trace
    if event == 'opcode' and frame.f_code is f.__code__:
        if frame.f_code.co_code[frame.f_lasti] == dis.opmap['NOT_TAKEN']:
            raise ValueError('trace')
    return trace

sys.settrace(trace)
f(False)
sys.settrace(None)
try:
    sys.settrace(trace)
    f(True)
except ValueError:
    print('propagated')
else:
    print('swallowed')
finally:
    sys.settrace(None)
";
    common::assert_executes(source, "propagated\n");
}

#[test]
fn nested_finally_call_marker_does_not_catch_trace_exception() {
    let source = r"import dis
import sys

def consume(value):
    return value

def f(flag):
    for _ in range(2):
        try:
            try:
                consume('body')
            finally:
                try:
                    consume('finally')
                    consume(flag)
                    break
                finally:
                    if consume(flag):
                        consume(1)
        except Exception:
            consume('except')

def trace(frame, event, arg):
    if event == 'call' and frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        frame.f_trace_lines = False
        return trace
    if event == 'opcode' and frame.f_code is f.__code__:
        if frame.f_code.co_code[frame.f_lasti] == dis.opmap['NOT_TAKEN']:
            raise ValueError('trace')
    return trace

sys.settrace(trace)
f(False)
sys.settrace(None)
try:
    sys.settrace(trace)
    f(True)
except ValueError:
    print('propagated')
else:
    print('swallowed')
finally:
    sys.settrace(None)
";
    common::assert_executes(source, "propagated\n");
    let bytecode_source = r"def f(log, flag):
    for i in range(2):
        try:
            try:
                consume('body')
            finally:
                try:
                    consume('finally')
                    consume(flag)
                    break
                finally:
                    if consume(flag):
                        consume(1)
        except Exception:
            consume('except')
    return log
";
    common::assert_matches_cpython_marshal(bytecode_source, "nested_finally_call_marker.py");
}

#[test]
fn finally_break_keeps_enclosing_handler_for_manager_exit() {
    let source = r"class Manager:
    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, traceback):
        raise ValueError('exit failed')

def f(items):
    for item in items:
        try:
            with Manager():
                try:
                    raise RuntimeError('work failed')
                finally:
                    break
        except ValueError:
            print('caught')

f([1])
";
    common::assert_executes(source, "caught\n");
    common::assert_matches_cpython_marshal(source, "finally_break_manager_exit_error.py");
}

#[test]
fn finally_break_keeps_enclosing_handler_for_outer_cleanup() {
    let source = r"def cleanup():
    print('cleanup')
    raise ValueError('cleanup failed')

for item in range(1):
    try:
        try:
            try:
                raise RuntimeError('work failed')
            finally:
                break
        finally:
            cleanup()
    except ValueError:
        print('caught')
";
    common::assert_executes(source, "cleanup\ncaught\n");
    common::assert_matches_cpython_marshal(source, "finally_break_outer_cleanup_error.py");
}

#[test]
fn finally_break_keeps_enclosing_handler_for_nested_cleanup() {
    let source = r"def cleanup():
    raise ValueError('cleanup failed')

for i in range(1):
    try:
        try:
            pass
        finally:
            try:
                break
            finally:
                cleanup()
    except ValueError:
        print('caught')
";
    common::assert_executes(source, "caught\n");
    common::assert_matches_cpython_marshal(source, "finally_break_nested_cleanup_error.py");
}

#[test]
fn nested_finally_continues_restore_exception_state_after_suspension() {
    for (function, suspension, driver) in [
        ("def", "pass", "target()"),
        ("def", "yield item", "list(target())"),
        (
            "async def",
            "await asyncio.sleep(0)",
            "asyncio.run(target())",
        ),
        (
            "async def",
            "yield item",
            "async def collect():\n    async for _ in target():\n        pass\nasyncio.run(collect())",
        ),
    ] {
        let source = format!(
            r"import asyncio
import sys

events = []
def observe(where):
    error = sys.exception()
    events.append((where, None if error is None else (type(error).__name__, error.args)))

class Manager:
    def __enter__(self):
        observe('enter')
    def __exit__(self, *exc):
        observe('exit')
        raise LookupError('manager')

{function} target():
    for item in [1, 2]:
        try:
            with Manager():
                try:
                    raise ValueError(item)
                finally:
                    try:
                        observe('before')
                        continue
                    finally:
                        {suspension}
                        observe('during')
                        continue
        except LookupError:
            observe('caught')

{driver}
print(events)
"
        );
        common::assert_matches_cpython_marshal(&source, "nested_finally_continue_state.py");
        common::assert_executes(
            &source,
            concat!(
                "[('enter', None), ('before', ('ValueError', (1,))), ",
                "('during', ('ValueError', (1,))), ('exit', None), ",
                "('caught', ('LookupError', ('manager',))), ('enter', None), ",
                "('before', ('ValueError', (2,))), ('during', ('ValueError', (2,))), ",
                "('exit', None), ('caught', ('LookupError', ('manager',)))]\n",
            ),
        );
    }
}

#[test]
fn nested_finally_condition_marker_exception_coverage() {
    for (function, condition, result) in [
        ("async def", "condition", "[make()]"),
        ("def", "condition", "[make()]"),
        ("async def", "not condition", "[make()]"),
        ("async def", "condition.flag", "[make()]"),
        ("async def", "condition", "[]"),
        ("async def", "condition", "1"),
        ("async def", "condition is None", "[make()]"),
        ("async def", "bool(condition)", "[make()]"),
    ] {
        let source = format!(
            r"{function} example(condition):
    try:
        pass
    except Exception as error:
        if not error.flag:
            pass
        return {result}
    finally:
        try:
            if {condition}:
                pass
        finally:
            pass
"
        );
        common::assert_matches_cpython_marshal(&source, "nested_finally_marker.py");
    }
}

#[test]
fn nested_finally_marker_control_flow_boundaries() {
    let source = r"async def example(condition, other):
    try:
        pass
    except Exception as error:
        if not error.flag:
            pass
        return [make()]
    finally:
        try:
            if condition:
                pass
        finally:
            pass
";
    for source in [
        source.replace(
            "            if condition:\n                pass",
            "            if condition:\n                pass\n            if other:\n                pass",
        ),
        source.replace(
            "            if condition:\n                pass",
            "            if condition:\n                pass\n            try:\n                observe()\n            except LookupError:\n                pass",
        ),
        source.replace(
            "        return [make()]",
            "        if other:\n            return []\n        return [make()]",
        ),
        source.replace(
            "        if not error.flag:\n            pass",
            "        if not error.flag: pass",
        ),
    ] {
        common::assert_matches_cpython_marshal(&source, "nested_finally_boundaries.py");
    }
}

#[test]
fn nested_finally_condition_marker_matches_each_target() {
    let source = r"async def example(condition):
    try:
        pass
    except Exception as error:
        if not error.flag:
            pass
        return [make()]
    finally:
        try:
            if condition:
                pass
        finally:
            pass
";
    assert_finally_condition_marker_matches_each_target(source);
}

fn assert_finally_condition_marker_matches_each_target(source: &str) {
    use serc::{CompileOptions, compile_with_options};

    let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), 'nested_finally_targets.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    for version in crate::common::cpython::VERSIONS {
        let target = version.target();
        let expected = crate::common::cpython::run_with_input(
            crate::common::cpython::command(version)
                .args(["-I", "-S", "-B", "-c", oracle])
                .args([target.tag.trim_start_matches('v'), &target.magic_hex()]),
            source.as_bytes(),
        );
        assert!(
            expected.status.success(),
            "{version:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        let actual = compile_with_options(source, "nested_finally_targets.py", &options)
            .unwrap()
            .marshal();
        assert_eq!(actual, expected.stdout, "{version:?}");
    }
}

#[test]
fn branching_try_nested_finally_condition_marker() {
    let source = r"async def example(condition, other):
    try:
        if other:
            raise ValueError()
        pass
    except Exception as error:
        if not error.flag:
            pass
        return [make()]
    finally:
        try:
            if condition:
                pass
        finally:
            pass
";
    assert_finally_condition_marker_matches_each_target(source);
    for source in [
        source.replace("async def", "def"),
        source.replace("if condition:", "if not condition:"),
        source.replace("if condition:", "if condition.flag:"),
        source.replace("return [make()]", "return []"),
        source.replace("return [make()]", "return 1"),
        source.replace("raise ValueError()", "return []"),
        source.replace("raise ValueError()", "observe()"),
        source.replace("        pass\n    except", "        observe()\n    except"),
        source.replace("        pass\n    except", "    except"),
        source.replace("if other:", "if False:"),
        source.replace("if other:", "if True:"),
    ] {
        common::assert_matches_cpython_marshal(&source, "branching_try_finally_marker.py");
    }
}

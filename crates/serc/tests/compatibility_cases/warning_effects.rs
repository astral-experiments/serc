use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompilationWarningEffect, CompileOptions, PreparedCompilation, PythonVersion};

use crate::common;

// The bootstrap is identical in both runs. The marker arrives through argv, so compiling
// the callback cannot itself intern it. All warning machinery is initialized before the
// tested compilation; the callback either retains one canonical string or changes nothing.
const BOOTSTRAP: &str = r"
import json, marshal, sys, warnings
retained = []
seen = []

def observe(message, category, filename, lineno, file=None, line=None):
    seen.append([str(message), category.__name__])
    if sys.argv[3] == 'retain':
        retained.append(sys.intern(sys.argv[2]))

warnings.showwarning = observe
warnings.simplefilter('always')
code = compile(sys.stdin.buffer.read(), sys.argv[1], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
sys.stderr.write(json.dumps(seen))
";

fn compare_in_context(source: &str, marker: &str, retain: bool) -> Vec<u8> {
    compare_in_context_for(source, marker, retain, PythonVersion::Py314)
}

fn compare_in_context_for(
    source: &str,
    marker: &str,
    retain: bool,
    version: PythonVersion,
) -> Vec<u8> {
    let filename = "warning_effects_context.py";
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    let prepared = PreparedCompilation::bytes(source.as_bytes(), filename, &options).unwrap();
    let mut effects = prepared.effects();
    for warning in prepared.warnings() {
        let effect = if retain {
            CompilationWarningEffect::RetainedStrings(vec![marker.to_string()])
        } else {
            CompilationWarningEffect::NoRetainedAdditions
        };
        effects.record(warning.ordinal(), effect).unwrap();
    }
    let actual = prepared.finish(&effects).unwrap().marshal();
    let mut python = if version == PythonVersion::Py314 {
        common::python_3145_command()
    } else {
        let (variable, fallback) = match version {
            PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12"),
            PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13"),
            PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
            PythonVersion::Py314 => unreachable!(),
        };
        let executable = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let identity = Command::new(&executable)
            .args(["-I", "-S", "-c", "import importlib.util, sys; print(sys.implementation.name, sys.version.split()[0], importlib.util.MAGIC_NUMBER.hex())"])
            .output().unwrap();
        let target = version.target();
        let expected = format!(
            "cpython {} {}\n",
            target.tag.strip_prefix('v').unwrap(),
            target.magic_hex()
        );
        assert!(identity.status.success());
        assert_eq!(String::from_utf8(identity.stdout).unwrap(), expected);
        let mut command = Command::new(executable);
        command.args(["-I", "-S", "-X", "utf8"]);
        command
    };
    let mut child = python
        .arg("-B")
        .args([
            "-c",
            BOOTSTRAP,
            filename,
            marker,
            if retain { "retain" } else { "quiet" },
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let observed: Vec<(String, String)> = serde_json::from_slice(&output.stderr).unwrap();
    let expected: Vec<_> = prepared
        .warnings()
        .iter()
        .map(|warning| {
            (
                warning.message().to_string(),
                warning.category().to_string(),
            )
        })
        .collect();
    assert_eq!(observed, expected, "{source}");
    assert_eq!(actual, output.stdout, "retain={retain}: {source}");
    actual
}

#[test]
fn warning_callbacks_preserve_native_phase_and_marshal_identity() {
    let first = "def first(value):\n    return value in {'warning_callback_marker_796', 'novel_callback_marker_796'}, 1\n";
    let second = "def second(value):\n    'second docstring'\n    return value in {'warning_callback_marker_796', 'novel_callback_marker_796'}, 2\n";
    let warning = "if 1 is 1:\n    pass\n";
    for (source, sensitive) in [
        (format!("{warning}{first}{second}"), true),
        (format!("{first}{warning}{second}"), false),
        (format!("{first}{second}{warning}"), false),
        (format!("{first}{second}literal = '\\z'\n"), true),
    ] {
        let quiet = compare_in_context(&source, "warning_callback_marker_796", false);
        let retained = compare_in_context(&source, "warning_callback_marker_796", true);
        assert_eq!(quiet != retained, sensitive, "{source}");
    }
}

#[test]
fn warning_callbacks_resolve_qualified_names_at_assembly() {
    for source in [
        "if 1 is 1: pass\nclass WarningOuter796:\n    class Inner: pass\n",
        "class WarningOuter796:\n    class Inner:\n        if 1 is 1: pass\n",
        "class WarningOuter796:\n    class Inner: pass\nif 1 is 1: pass\n",
        "if False:\n    class WarningOuter796:\n        class Inner: pass\nif 1 is 1: pass\nclass WarningOuter796:\n    class Inner: pass\n",
    ] {
        compare_in_context(source, "WarningOuter796.Inner", false);
        compare_in_context(source, "WarningOuter796.Inner", true);
    }
}

#[test]
fn warning_callbacks_preserve_annotation_assembly_order() {
    for (source, marker) in [
        (
            "value: int\nif 1 is 1: pass\ndef f(arg: int): pass\n",
            "__annotate__",
        ),
        (
            "if 1 is 1: pass\nclass WarningAnnotated796:\n    value: int\n    def f(self) -> int:\n        if 1 is 1: pass\n    if 1 is 1: pass\n",
            "WarningAnnotated796.__annotate__",
        ),
        (
            "def outer():\n    if 1 is 1: pass\n    @decorator\n    def inner(arg: int):\n        if 1 is 1: pass\n    @decorator\n    class WarningAnnotated796:\n        value: int\n        if 1 is 1: pass\n",
            "outer.<locals>.WarningAnnotated796.__annotate__",
        ),
    ] {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            compare_in_context_for(source, marker, false, version);
            compare_in_context_for(source, marker, true, version);
        }
    }
    let first = "def first(value):\n    return value in {'warning_callback_marker_796', 'novel_callback_marker_796'}, 1\n";
    let second = "def second(value):\n    'second docstring'\n    return value in {'warning_callback_marker_796', 'novel_callback_marker_796'}, 2\n";
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (source, sensitive) in [
            (
                format!("value: int\nif 1 is 1: pass\n{first}{second}"),
                true,
            ),
            (
                format!("{first}\ndef annotated(value: int): pass\nif 1 is 1: pass\n{second}"),
                false,
            ),
            (
                format!("{first}{second}\nvalue: int\nif 1 is 1: pass\n"),
                false,
            ),
        ] {
            let quiet =
                compare_in_context_for(&source, "warning_callback_marker_796", false, version);
            let retained =
                compare_in_context_for(&source, "warning_callback_marker_796", true, version);
            assert_eq!(quiet != retained, sensitive, "{version:?}: {source}");
        }
    }
}

#[test]
fn warning_callbacks_preserve_assembly_lines_with_mixed_newlines() {
    let source = "@decorator\r\ndef outer():\r    def inner():\n        return 1 is x\r\n    return inner\r\n";
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        compare_in_context_for(source, "mixed_newline_marker_796", false, version);
        compare_in_context_for(source, "mixed_newline_marker_796", true, version);
    }
}

#[test]
fn warning_callbacks_preserve_lines_for_text_and_bytes_sources() {
    for newline in ["\n", "\r\n", "\r"] {
        let source = [
            "# π",
            "@decorator",
            "def outer():",
            "    def inner():",
            "        return 1 is x",
            "    return inner",
        ]
        .join(newline);
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let expected = compare_in_context_for(&source, "line_marker_796", false, version);
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            for prepared in [
                PreparedCompilation::text(&source, "warning_effects_context.py", &options),
                PreparedCompilation::bytes(
                    source.as_bytes(),
                    "warning_effects_context.py",
                    &options,
                ),
            ] {
                let prepared = prepared.unwrap();
                let mut effects = prepared.effects();
                for warning in prepared.warnings() {
                    effects
                        .record(
                            warning.ordinal(),
                            CompilationWarningEffect::NoRetainedAdditions,
                        )
                        .unwrap();
                }
                assert_eq!(prepared.finish(&effects).unwrap().marshal(), expected);
            }
        }
    }
}

#[test]
fn unknown_warning_callbacks_preserve_shared_frozenset_owners() {
    let marker = "warning_shared_owner_796";
    let first = "def first(value):\n    return value in {'warning_shared_owner_796', 'warning_shared_companion_796'}\n";
    let second = "def second(value):\n    return value in {'warning_shared_owner_796', 'warning_shared_companion_796'}\n";
    let warning = "if 1 is 1: pass\n";
    for source in [
        format!("{warning}{first}{second}"),
        format!("{first}{warning}{second}"),
        format!("{first}{second}{warning}"),
    ] {
        let quiet = compare_in_context_for(&source, marker, false, PythonVersion::Py315);
        let retained = compare_in_context_for(&source, marker, true, PythonVersion::Py315);
        assert_eq!(quiet, retained, "{source}");

        let options = CompileOptions {
            python_version: PythonVersion::Py315,
            ..CompileOptions::default()
        };
        let prepared =
            PreparedCompilation::bytes(source.as_bytes(), "warning_effects_context.py", &options)
                .unwrap();
        assert_eq!(prepared.warnings().len(), 1);
        let mut effects = prepared.effects();
        effects
            .record(
                prepared.warnings()[0].ordinal(),
                CompilationWarningEffect::PartialRetainedStrings(vec![]),
            )
            .unwrap();
        assert_eq!(
            prepared.finish(&effects).unwrap().marshal(),
            quiet,
            "{source}"
        );
    }

    let second_distinct = "def second(value):\n    'different tuple'\n    return value in {'warning_shared_owner_796', 'warning_shared_companion_796'}\n";
    let source = format!("{warning}{first}{second_distinct}");
    let quiet = compare_in_context_for(&source, marker, false, PythonVersion::Py315);
    let retained = compare_in_context_for(&source, marker, true, PythonVersion::Py315);
    assert_ne!(quiet, retained);
    let options = CompileOptions {
        python_version: PythonVersion::Py315,
        ..CompileOptions::default()
    };
    let prepared =
        PreparedCompilation::bytes(source.as_bytes(), "warning_effects_context.py", &options)
            .unwrap();
    assert_eq!(prepared.warnings().len(), 1);
    let mut effects = prepared.effects();
    effects
        .record(
            prepared.warnings()[0].ordinal(),
            CompilationWarningEffect::PartialRetainedStrings(vec![]),
        )
        .unwrap();
    assert!(matches!(
        prepared.finish(&effects),
        Err(serc::CompileError::UnresolvedInternState { .. })
    ));
}

#[test]
fn warning_callbacks_preserve_discarded_class_scalar_origins() {
    let source = "if 1 is 1: pass\nif False:\n    class WarningOuter796:\n        class Inner: pass\nvalue = 'WarningOuter796.Inner'\n";
    let quiet = compare_in_context(source, "WarningOuter796.Inner", false);
    let retained = compare_in_context(source, "WarningOuter796.Inner", true);
    assert_ne!(quiet, retained);
    assert!(matches!(
        serc::compile(source, "warning_effects_context.py"),
        Err(serc::CompileError::UnresolvedInternState { .. })
    ));
}

#[test]
fn warning_formatting_codec_effects_are_bound_to_the_supplied_context() {
    // The compiler reads stdin while native warning formatting reads this fixed
    // filename. A codec cookie in that separate file can change canonical owners.
    const SOURCE: &str = "if 1 is 1: pass\ndef first(x): return x in {'encoding_table', 'warning_codec_marker'}, 1\ndef second(x):\n    'second docstring'\n    return x in {'encoding_table', 'warning_codec_marker'}, 2\n";
    const STANDARD_WARNING: &str = r"
import marshal, sys
code = compile(sys.stdin.buffer.read(), sys.argv[1], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let directory = tempfile::tempdir().unwrap();
    let filename = directory.path().join("warning_codec.py");
    let filename_text = filename.to_str().unwrap();
    let native = || {
        let mut child = common::python_3145_command()
            .arg("-B")
            .args(["-c", STANDARD_WARNING, filename_text])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(SOURCE.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("SyntaxWarning"));
        output.stdout
    };
    fs_err::write(&filename, "# coding: utf-8\n").unwrap();
    let utf8 = native();
    fs_err::write(&filename, "# coding: cp1251\n").unwrap();
    let cp1251 = native();
    assert_ne!(utf8, cp1251);

    let prepared =
        PreparedCompilation::bytes(SOURCE.as_bytes(), filename_text, &CompileOptions::default())
            .unwrap();
    let mut effects = prepared.effects();
    for warning in prepared.warnings() {
        // encodings.cp1251's constructed code owns this canonical identifier;
        // the rest of standard warning formatting remains deliberately unknown.
        effects
            .record(
                warning.ordinal(),
                CompilationWarningEffect::PartialRetainedStrings(vec![
                    "encoding_table".to_string(),
                ]),
            )
            .unwrap();
    }
    assert_eq!(prepared.finish(&effects).unwrap().marshal(), cp1251);
}

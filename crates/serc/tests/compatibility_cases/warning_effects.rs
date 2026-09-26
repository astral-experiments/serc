use std::io::Write as _;
use std::process::Stdio;

use serc::{CompilationWarningEffect, CompileOptions, PreparedCompilation};

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
    let filename = "warning_effects_context.py";
    let prepared =
        PreparedCompilation::bytes(source.as_bytes(), filename, &CompileOptions::default())
            .unwrap();
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
    let mut child = common::python_3145_command()
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

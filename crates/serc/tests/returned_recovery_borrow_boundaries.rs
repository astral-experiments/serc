use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython(source: &str) {
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let filename = "returned_recovery_borrow_boundaries.py";
        let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
        let mut child = Command::new(python)
            .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
            .args([
                target.tag.trim_start_matches('v'),
                &target.magic_hex(),
                filename,
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
        let expected = child.wait_with_output().unwrap();
        assert!(
            expected.status.success(),
            "{version:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for actual in [
            compile_with_options(source, filename, &options).unwrap(),
            compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
        ] {
            assert_eq!(actual.marshal(), expected.stdout, "{version:?}:\n{source}");
        }
    }
}

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

fn recovery(asynchronous: bool, otherwise: bool, binding: &str) -> String {
    let awaited = if asynchronous { "await " } else { "" };
    let returned = if otherwise {
        ""
    } else {
        "        return value\n"
    };
    let otherwise = if otherwise {
        "    else:\n        return value\n"
    } else {
        ""
    };
    format!(
        "try:\n    value = {awaited}work(other)\nexcept Error{binding}:\n    if skip:\n        raise\n    try:\n        {awaited}recover(value)\n{returned}    except Inner{binding}:\n        if flag:\n            raise\n{otherwise}    value = {awaited}finish(other)\n"
    )
}

fn function(asynchronous: bool, body: &str) -> String {
    let asynchronous = if asynchronous { "async " } else { "" };
    format!(
        "{asynchronous}def f(value, other, flag, skip):\n{}",
        indent(body)
    )
}

#[test]
fn returned_handler_recoveries_preserve_the_original_normal_cleanup_boundary() {
    for asynchronous in [false, true] {
        for otherwise in [false, true] {
            for binding in ["", " as error"] {
                let body = recovery(asynchronous, otherwise, binding)
                    + "finish(value, other, flag)\nreturn value\n";
                assert_matches_cpython(&function(asynchronous, &body));
                assert_matches_cpython(&function(
                    asynchronous,
                    &body.replace("except Inner", "except (Inner, Other)"),
                ));
            }
        }
    }
}

#[test]
fn returned_recovery_exits_respect_the_original_copy_width() {
    for asynchronous in [false, true] {
        for tail in [
            "return value",
            "return None",
            "raise Error()",
            "raise Error(value)",
            "raise Error(value, other)",
            "return finish(value)",
            "if flag:\n    finish(value)\nreturn other",
            "assert value\nreturn other",
        ] {
            let body = recovery(asynchronous, false, " as error") + tail;
            assert_matches_cpython(&function(asynchronous, &body));
        }
    }
}

#[test]
fn returned_recovery_barriers_preserve_independent_continuation_entries() {
    for asynchronous in [false, true] {
        for bypass in ["pass", "prepare(value)"] {
            let guarded = recovery(asynchronous, false, " as error");
            let body = format!(
                "if skip:\n    {bypass}\nelse:\n{}finish(value, other, flag)\nreturn value\n",
                indent(&guarded),
            );
            assert_matches_cpython(&function(asynchronous, &body));
        }
    }
}

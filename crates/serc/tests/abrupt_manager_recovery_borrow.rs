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
        let filename = "abrupt_manager_recovery_borrow.py";
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

fn recovery(outer: &str, binding: &str, prefix: &str, exit: &str) -> String {
    let handler = indent(&indent(&indent(&(prefix.to_owned() + exit + "\n"))));
    format!(
        "try:\n    work(value)\nexcept Error{outer}:\n    with manager(value){binding}:\n        try:\n            retry(value)\n        except Inner as error:\n{handler}        completed(value)\n    recover(value)\n"
    )
}

fn function(asynchronous: bool, body: &str) -> String {
    let asynchronous = if asynchronous { "async " } else { "" };
    format!("{asynchronous}def f(value, other, flag):\n{}", indent(body))
}

#[test]
fn abrupt_named_manager_recoveries_keep_the_original_empty_cleanup_boundary() {
    for asynchronous in [false, true] {
        for outer in ["", " as outer"] {
            for binding in ["", " as owner"] {
                for prefix in [
                    "",
                    "if flag:\n    prepare(value)\n",
                    "try:\n    check(value)\nexcept Retry as retry:\n    raise error from retry\n",
                ] {
                    for exit in ["return value", "raise Other(value) from error"] {
                        let body = recovery(outer, binding, prefix, exit)
                            + "finish(value, flag)\nreturn value\n";
                        assert_matches_cpython(&function(asynchronous, &body));
                    }
                }
            }
        }
    }
}

#[test]
fn abrupt_manager_boundaries_respect_original_copy_width_and_independent_entries() {
    for asynchronous in [false, true] {
        for tail in [
            "return value\n",
            "return factory()\n",
            "return factory(value)\n",
            "if flag:\n    return value\nfinish(value)\nreturn other\n",
            "finish(value, flag)\nfinish(other)\nreturn value\n",
        ] {
            let body = recovery("", "", "", "return value") + tail;
            assert_matches_cpython(&function(asynchronous, &body));
        }
        for bypass in ["pass", "prepare(value)"] {
            let body = format!(
                "if flag:\n{}else:\n    {bypass}\nfinish(value, flag)\nreturn value\n",
                indent(&recovery("", "", "", "return value")),
            );
            assert_matches_cpython(&function(asynchronous, &body));
        }
    }
}

#[test]
fn successive_abrupt_recoveries_retain_each_original_named_cleanup() {
    for asynchronous in [false, true] {
        let first = recovery(
            "",
            "",
            "if flag is not None:\n    prepare(value)\ntry:\n    check(value)\nexcept Retry as retry:\n    raise error from retry\ncompleted(value)\n",
            "return",
        );
        let body = first.replace(
            "\n        completed(value)\n",
            "\n        try:\n            state = other\n            retry(value)\n        except Other as second:\n            completed(value)\n            raise Error(value) from second\n        completed(value)\n",
        ) + "finish(value, flag)\nreturn value\n";
        assert_matches_cpython(&function(asynchronous, &body));
    }
}

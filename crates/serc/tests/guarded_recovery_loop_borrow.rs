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
        let filename = "guarded_recovery_loop_borrow.py";
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

fn function(asynchronous: bool, body: &str) -> String {
    let asynchronous = if asynchronous { "async " } else { "" };
    format!(
        "{asynchronous}def f(value, flag, values, other):\n{}",
        indent(body)
    )
}

#[test]
fn guarded_pass_recoveries_leave_a_cold_array_between_warm_blocks() {
    for asynchronous in [false, true] {
        for guarded in [false, true] {
            for binding in ["", " as error"] {
                for leading in ["", "prepare(value)\n"] {
                    for recovery in ["pass", "recover(item)"] {
                        let loop_body = format!(
                            "for item in values:\n    try:\n        update(item)\n    except Inner:\n        {recovery}\nfinish_loop(value)\n"
                        );
                        let handler = leading.to_owned()
                            + &if guarded {
                                format!("if flag:\n{}", indent(&loop_body))
                            } else {
                                loop_body
                            };
                        let body = format!(
                            "try:\n    work(value)\nexcept Error{binding}:\n{}finish(value, flag)\nreturn value\n",
                            indent(&handler)
                        );
                        assert_matches_cpython(&function(asynchronous, &body));
                    }
                }
            }
        }
    }
}

#[test]
fn non_suspending_raising_prefixes_preserve_the_recovery_backedge() {
    for asynchronous in [false, true] {
        for condition in [
            "other",
            "other is None or flag",
            "flag and other",
            "not flag",
        ] {
            for raised in ["raise Inner(other)", "raise Inner(other) from None"] {
                let body = format!(
                    "try:\n    work(value)\nexcept Error as error:\n    prepare(value)\n    if flag:\n        if {condition}:\n            {raised}\n        for item in values:\n            prepare(item)\n            try:\n                update(item)\n            except Inner:\n                pass\n        finish_loop(value)\nfinish(value, flag, other)\n"
                );
                assert_matches_cpython(&function(asynchronous, &body));
            }
        }
    }
}

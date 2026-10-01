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
        let filename = "pending_try_finally_continue.py";
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

#[test]
fn pending_ordinary_tries_keep_each_copied_conditional_continue() {
    for scope in ["sync", "coroutine", "generator", "async-generator"] {
        for header in ["for value in values:", "while running:"] {
            for pending in 0..=2 {
                for condition in ["flag", "value is not None", "value > 0"] {
                    if pending > 0 && condition == "flag" {
                        continue;
                    }
                    for tail in ["", "tail(value)\n"] {
                        let mut body = format!(
                            "try:\n    if stop:\n        continue\n    work(value)\nfinally:\n    if {condition}:\n        clean(value)\n"
                        );
                        for depth in 0..pending {
                            body = format!(
                                "try:\n{}except Error{depth}:\n    recover(value)\n",
                                indent(&body)
                            );
                        }
                        body.push_str(tail);
                        body = format!("{header}\n{}", indent(&body));
                        body.push_str(match scope {
                            "generator" | "async-generator" => "yield value\n",
                            "coroutine" => "await finish(value)\n",
                            _ => "return value\n",
                        });
                        let prefix = if matches!(scope, "coroutine" | "async-generator") {
                            "async "
                        } else {
                            ""
                        };
                        assert_matches_cpython(&format!(
                            "{prefix}def f(values, value, flag, stop, running):\n{}",
                            indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

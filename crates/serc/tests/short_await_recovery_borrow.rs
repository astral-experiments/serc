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
        let filename = "short_await_recovery_borrow.py";
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
fn nested_else_return_preserves_short_await_borrow_boundaries() {
    for early in [false, true] {
        for outer in ["", " as error"] {
            for inner in ["", " as inner"] {
                for recovery in ["pass", "guard", "multiple"] {
                    for tail in [
                        "await value",
                        "await finish(value)",
                        "finish(value, other, ordered=True)",
                        "return value",
                    ] {
                        let mut handlers = format!("except Inner{inner}:\n");
                        handlers.push_str(if recovery == "pass" {
                            "    pass\n"
                        } else {
                            "    if flag:\n        raise\n"
                        });
                        if recovery == "multiple" {
                            handlers = "except Other:\n    pass\n".to_owned() + &handlers;
                        }
                        let nested = format!(
                            "try:\n    await wait(value)\n{handlers}else:\n    return\nfinish(value, other)\n"
                        );
                        let leading = if early {
                            "if flag:\n    await other\n    return\n"
                        } else {
                            ""
                        };
                        let body = format!(
                            "{leading}try:\n    await prepare(value)\nexcept Error{outer}:\n    touch(value)\n{}{tail}\n",
                            indent(&nested)
                        );
                        assert_matches_cpython(&format!(
                            "async def f(value, other, flag):\n{}",
                            indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

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
        let filename = "folded_manager_predecessor.py";
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

fn recovery(manager: &str, body: &str, handler: &str, suffix: &str) -> String {
    let protected = format!("{manager}:\n{}{suffix}", indent(body));
    format!(
        "try:\n{}except ValueError as error:\n{}",
        indent(&protected),
        indent(handler)
    )
}

#[test]
fn folded_inner_manager_keeps_the_original_shared_normal_exit() {
    for outer in ["", "with outer()", "async with outer()"] {
        for inner in ["with inner()", "async with inner()"] {
            for body in [
                "return value",
                "while True:\n    return value",
                "while flag:\n    return value",
                "while True:\n    break",
            ] {
                for handler in ["raise RuntimeError from error", "pass", "return None"] {
                    let suite = recovery(inner, body, handler, "");
                    let suite = if outer.is_empty() {
                        suite
                    } else {
                        format!("{outer}:\n{}", indent(&suite))
                    };
                    assert_matches_cpython(&format!(
                        "async def f(value, flag):\n{}",
                        indent(&suite)
                    ));
                }
            }
        }
    }
}

#[test]
fn original_manager_predecessors_stay_local_to_the_normal_cleanup() {
    for inner in ["with inner()", "async with inner()"] {
        for body in ["while True:\n    return value", "while True:\n    pass"] {
            for handler in ["raise RuntimeError from error", "return None"] {
                for context in [
                    "plain",
                    "outer pair",
                    "nested outer",
                    "loop",
                    "outer finally",
                    "successor",
                    "inner pair",
                ] {
                    let manager = if context == "inner pair" {
                        format!("{inner}, other()")
                    } else {
                        inner.to_owned()
                    };
                    let protected = recovery(
                        &manager,
                        body,
                        handler,
                        if context == "successor" {
                            "finish()\n"
                        } else {
                            ""
                        },
                    );
                    let outer = if context == "outer pair" {
                        "with outer(), other()"
                    } else {
                        "with outer()"
                    };
                    let suite = format!("{outer}:\n{}", indent(&protected));
                    let suite = match context {
                        "nested outer" => format!("with guard():\n{}", indent(&suite)),
                        "loop" => format!("for item in items:\n{}", indent(&suite)),
                        "outer finally" => {
                            format!("try:\n{}finally:\n    finish()\n", indent(&suite))
                        }
                        _ => suite,
                    };
                    assert_matches_cpython(&format!(
                        "async def f(value, flag):\n{}",
                        indent(&suite)
                    ));
                }
            }
        }
    }
}

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
        let filename = "completed_loop_try_exit_locations.py";
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
fn completed_loop_else_assignments_keep_their_original_normal_exit_nop() {
    for head in ["for item in items:", "while ready():"] {
        for tail in ["", "    work(value)\n"] {
            for value in [
                "finish()",
                "finish(\n        value,\n        flag,\n    )",
                "flag if value else items",
            ] {
                for continuation in ["consume(value)\nreturn value\n", "return value\n"] {
                    // Short copied return arrays have a separate original NOP producer.
                    if continuation == "return value\n"
                        && (head == "while ready():" || value == "flag if value else items")
                    {
                        continue;
                    }
                    let body = format!(
                        "{head}\n    if flag:\n        break\n{tail}else:\n    value = {value}\n"
                    );
                    let body = format!(
                        "try:\n{}except Error:\n    pass\n{continuation}",
                        indent(&body),
                    );
                    assert_matches_cpython(&format!(
                        "def f(items, flag, value, manager):\n{}",
                        indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn completed_loop_exit_certificates_keep_cleanup_and_nested_breaks_separate() {
    for context in ["plain", "async", "with", "async-with", "finally"] {
        for head in ["for item in items:", "while ready():"] {
            for transfer in ["break", "return", "nested-break", "none"] {
                for value in [
                    "finish()",
                    "flag if value else items",
                    "collect(item for item in items)",
                ] {
                    if transfer != "break" && (head == "while ready():" || value != "finish()") {
                        continue;
                    }
                    let body = match transfer {
                        "break" => "if flag:\n    break\nwork(value)\n",
                        "return" => "if flag:\n    return value\nwork(value)\n",
                        "nested-break" => {
                            "for other in items:\n    if flag:\n        break\nwork(value)\n"
                        }
                        _ => "work(value)\n",
                    };
                    let asynchronous = matches!(context, "async" | "async-with");
                    let value = if asynchronous && value == "finish()" {
                        "await finish()"
                    } else {
                        value
                    };
                    let body = format!("{head}\n{}else:\n    value = {value}\n", indent(body));
                    let mut body = format!("try:\n{}except Error:\n    pass\n", indent(&body));
                    if context == "finally" {
                        body.push_str("finally:\n    cleanup(value)\n");
                    }
                    body.push_str("consume(value)\nreturn value\n");
                    if matches!(context, "with" | "async-with") {
                        let keyword = if asynchronous { "async with" } else { "with" };
                        body = format!("{keyword} manager:\n{}", indent(&body));
                    }
                    let prefix = if asynchronous { "async " } else { "" };
                    assert_matches_cpython(&format!(
                        "{prefix}def f(items, flag, value, manager):\n{}",
                        indent(&body)
                    ));
                }
            }
        }
    }
}

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
        let filename = "early_return_recovery_borrow.py";
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

fn recovery(leading: &str, inner: &str, outer: &str) -> String {
    format!(
        "try:\n{}    try:\n        result = work(value, key=other)\n    except Inner{inner}:\n        if flag:\n            result = recover(other)\n        else:\n            result.pop('value', None)\n    else:\n        if other is not None:\n            pass\nexcept Error{outer}:\n    return 2\n",
        indent(leading)
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
fn earlier_protected_returns_keep_nested_else_joins_empty() {
    for asynchronous in [false, true] {
        for leading in ["", "if skip:\n    return 0\n"] {
            for inner in ["", " as inner"] {
                for outer in ["", " as error"] {
                    let body = recovery(leading, inner, outer) + "finish(result, ordered=True)\n";
                    assert_matches_cpython(&function(asynchronous, &body));
                    let guarded = body.replace(
                        "        if flag:",
                        "        if other is None or skip:\n            raise\n        if flag:",
                    );
                    assert_matches_cpython(&function(asynchronous, &guarded));
                }
            }
        }
    }
}

#[test]
fn independent_routes_and_original_copy_width_keep_their_borrowing() {
    for asynchronous in [false, true] {
        for route in ["ordinary", "unconditional", "closed", "independent"] {
            for tail in [
                "return result",
                "return done()",
                "return done(result)",
                "finish(result)",
            ] {
                let leading = match route {
                    "unconditional" => "return value\n",
                    "closed" => "if skip:\n    return value\nelse:\n    return other\n",
                    _ => "if skip:\n    return 0\n",
                };
                let mut body = recovery(leading, "", " as error");
                if route == "independent" {
                    body = format!("if flag:\n    result = value\nelse:\n{}", indent(&body));
                }
                body.push_str(tail);
                assert_matches_cpython(&function(asynchronous, &body));
            }
        }
    }
}

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
        let filename = "shared_loop_assertion_exits.py";
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

fn function(body: &str) -> String {
    format!(
        "def check(items, flag, other, a, b, c, d, e, f, obj):\n{}",
        indent(body)
    )
}

#[test]
fn chained_assertion_successes_share_the_nonfinal_loop_exit() {
    for expression in [
        "not (a < b < c and d < e)",
        "not (a < b < c and d < e and e < f)",
        "not (a < b < c < d and e)",
        "a < b < c or d < e",
        "not (a < b < c and d is e)",
        "not (a < b < c and test(d) < e)",
        "not (a < test(b) < c and obj.value < other[0])",
        "not (a < b < c and (d or e))",
        "not (a < (b := test()) < c and d)",
        "not (a < (lambda value=b: value) < c and d)",
        "not (a < b < c and d)",
    ] {
        let selection =
            format!("if flag:\n    assert {expression}\nelse:\n    assert {expression}\n");
        for header in ["for item in items:", "while other:"] {
            let body = format!("{header}\n{}", indent(&selection));
            assert_matches_cpython(&function(&body));
        }
        let body = format!(
            "for outer in items:\n    for item in items:\n{}",
            indent(&indent(&selection))
        );
        assert_matches_cpython(&function(&body));
    }
}

#[test]
fn final_assertions_and_populated_loop_continuations_keep_their_own_layout() {
    for expression in [
        "not (a < b < c and d < e)",
        "a < b < c or d < e",
        "not (a < b and d < e)",
    ] {
        for body in [
            format!("for item in items:\n    assert {expression}\n"),
            format!("for item in items:\n    if flag:\n        assert {expression}\n"),
            format!(
                "for item in items:\n    if flag:\n        work()\n    else:\n        assert {expression}\n"
            ),
            format!(
                "for item in items:\n    if flag:\n        assert {expression}\n    else:\n        work()\n    work()\n"
            ),
            format!("if flag:\n    assert {expression}\nelse:\n    assert {expression}\n"),
        ] {
            assert_matches_cpython(&function(&body));
        }
    }
}

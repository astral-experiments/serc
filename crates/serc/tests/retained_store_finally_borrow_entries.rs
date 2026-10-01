use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

const SOURCE: &str = "def sample(obj, left, right):
    try:
        healthy = left and right
    except Exception as error:
        if obj.failed:
            raise
        report(error)
    finally:
        with obj.lock:
            obj.close()
        obj.finish()
";

fn assert_matches_cpython(source: &str) {
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let filename = "retained_store_finally_borrow_entries.py";
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

#[test]
fn retained_store_finally_entries_keep_both_boolean_edges_exact() {
    for operator in ["and", "or"] {
        for handler_test in ["obj.failed", "left == right", "not left and obj.failed"] {
            for prefix in ["", "        value = obj.prepare()\n"] {
                for suffix in ["obj.finish()", "if obj.ready:\n            obj.finish()"] {
                    let source = SOURCE
                        .replace("left and right", &format!("left {operator} right"))
                        .replace("if obj.failed:", &format!("if {handler_test}:"))
                        .replace("        healthy =", &format!("{prefix}        healthy ="))
                        .replace("obj.finish()", suffix);
                    assert_matches_cpython(&source);
                }
            }
        }
    }
}

#[test]
fn retained_store_finally_entries_preserve_upstream_visit_barriers() {
    for statement in [
        "assert False",
        "assert True",
        "assert obj.ready",
        "if obj.ready:\n        pass",
    ] {
        assert_matches_cpython(&SOURCE.replacen(
            "    try:\n",
            &format!("    {statement}\n    try:\n"),
            1,
        ));
    }
    assert_matches_cpython(&SOURCE.replace(
        "left and right",
        "obj.check(left) == 1 and right is not None",
    ));
}

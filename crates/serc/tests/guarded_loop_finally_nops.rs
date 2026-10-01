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
        let filename = "guarded_loop_finally_nops.py";
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
fn completed_cleanup_loops_keep_the_original_pop_line_and_borrow_barrier() {
    let recovery = "try:\n    work(item)\nexcept Exception as caught:\n    if error is None:\n        error = caught\n";
    for (asynchronous, operation) in [
        (false, "work(value)"),
        (false, "yield value"),
        (true, "await work(value)"),
        (true, "yield value"),
    ] {
        for loop_body in [
            recovery.to_owned(),
            "with manager:\n    work(item)\n".to_owned(),
            "work(item)\n".to_owned(),
            "if flag:\n    work(item)\nelse:\n    work(value)\n".to_owned(),
            format!("{recovery}work(item)\n"),
        ] {
            for finalizer in ["finish(value)", "target.finish(value)"] {
                for additional_recovery in [false, true] {
                    let mut cleanup =
                        format!("error = None\nfor item in values:\n{}", indent(&loop_body));
                    if additional_recovery {
                        cleanup.push_str("try:\n    work(value)\nexcept Exception as caught:\n    if error is None:\n        error = caught\n");
                    }
                    cleanup.push_str("if error is not None:\n    raise error\n");
                    let inner = format!("try:\n    {operation}\nfinally:\n{}", indent(&cleanup));
                    let body = format!("try:\n{}finally:\n    {finalizer}\n", indent(&inner));
                    assert_matches_cpython(&format!(
                        "{}def sample(value, values, flag, manager, target):\n{}",
                        if asynchronous { "async " } else { "" },
                        indent(&body)
                    ));
                }
            }
        }
    }
}

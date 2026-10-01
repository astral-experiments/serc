use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

const SOURCE: &str = "async def sample(resource, operation, cleanup, record, condition):
    failed = False
    result = None
    try:
        if condition == 1:
            await operation(resource)
        elif condition == 2:
            record(resource)
    except BaseException:
        failed = True
        await record(resource)
        raise
    finally:
        try:
            await cleanup(resource, resource.value, result)
        except Exception:
            raise
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
        let filename = "independent_finally_borrow_entries.py";
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
fn independent_awaited_entries_preserve_original_assertion_barriers() {
    assert_matches_cpython(SOURCE);
    for assertion in [
        "assert False",
        "assert True",
        "assert condition",
        "record(resource)",
    ] {
        assert_matches_cpython(&SOURCE.replacen(
            "            await operation(resource)",
            &format!("            {assertion}\n            await operation(resource)"),
            1,
        ));
        assert_matches_cpython(&SOURCE.replacen(
            "    try:\n",
            &format!("    {assertion}\n    try:\n"),
            1,
        ));
    }
}

#[test]
fn independent_awaited_entries_keep_call_headers_and_widths_exact() {
    for callable in ["cleanup", "close_resource", "resource.cleanup"] {
        for arguments in 0..5 {
            let arguments = vec!["resource"; arguments].join(", ");
            assert_matches_cpython(&SOURCE.replace(
                "cleanup(resource, resource.value, result)",
                &format!("{callable}({arguments})"),
            ));
        }
    }
}

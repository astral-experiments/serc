use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython(source: &str) {
    let filename = "copied_async_finally_ownership.py";
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let target = version.target();
        let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
        let mut child = Command::new(python)
            .args(["-B", "-I", "-S", "-c", oracle])
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
fn completed_manager_unwinds_preserve_the_original_finally_pop_cell() {
    for managers in ["other", "other, third"] {
        for returned in ["value", "produce(value)"] {
            for raised in ["raise Error()", "raise Error() from None"] {
                for cleanup in ["cleanup(value)", "await cleanup(value)"] {
                    let source = format!(
                        "async def run(value, manager, other, third, first, prepare, produce, cleanup, finish, flag):\n\
                         \x20   try:\n\
                         \x20       async with first:\n\
                         \x20           await prepare()\n\
                         \x20   except Error:\n\
                         \x20       {raised}\n\
                         \x20   value = produce()\n\
                         \x20   try:\n\
                         \x20       with manager:\n\
                         \x20           async with {managers}:\n\
                         \x20               return {returned}\n\
                         \x20   except Error:\n\
                         \x20       finish()\n\
                         \x20   finally:\n\
                         \x20       if value is not None:\n\
                         \x20           {cleanup}\n\
                         \x20       finish()\n\
                         \x20       if flag:\n\
                         \x20           finish()\n"
                    );
                    assert_matches_cpython(&source);
                }
            }
        }
    }
}

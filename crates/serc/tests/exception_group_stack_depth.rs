use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_with_options};

fn assert_matches_cpython(source: &str, version: PythonVersion) {
    let target = version.target();
    let (variable, fallback) = match version {
        PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12"),
        PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13"),
        PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
        PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
    };
    let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "exception_group_stack_depth.py";
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
    assert_eq!(
        compile_with_options(source, filename, &options)
            .unwrap()
            .marshal(),
        expected.stdout,
        "{version:?}: {source}"
    );
}

fn indent(source: &str) -> String {
    let mut indented = String::new();
    for line in source.lines() {
        writeln!(indented, "    {line}").unwrap();
    }
    indented
}

#[test]
fn later_group_handlers_retain_the_complete_subgroup_stack() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for preceding in 0..=2 {
            for name in ["", " as error"] {
                for earlier_body in ["pass", "raise"] {
                    for body in [
                        "await work(value)",
                        "await work(value, 'first' if flag else 'second')",
                        "async for item in values:\n    await work(item)",
                        "work(value, value, value, value, value, value, value, value)",
                    ] {
                        let mut suite = String::from("try:\n    await work(value)\n");
                        for index in 0..preceding {
                            writeln!(suite, "except* First{index}:\n    {earlier_body}").unwrap();
                        }
                        write!(suite, "except* Error{name}:\n{}", indent(body)).unwrap();
                        for suite in [
                            suite.clone(),
                            format!("with manager():\n{}", indent(&suite)),
                        ] {
                            let source =
                                format!("async def f(value, flag, values):\n{}", indent(&suite));
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

use std::ffi::OsString;
use std::io::Write;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_with_options};

pub(crate) fn targets() -> [(PythonVersion, OsString); 4] {
    [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ]
    .map(|(version, variable, fallback)| {
        (
            version,
            std::env::var_os(variable).unwrap_or_else(|| fallback.into()),
        )
    })
}

pub(crate) fn assert_matches_cpython(
    version: PythonVersion,
    executable: &OsString,
    name: &str,
    source: &str,
) -> Vec<u8> {
    const ORACLE: &str = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), 'private_type_parameters.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    let target = version.target();
    let mut child = Command::new(executable)
        .args(["-I", "-S", "-c", ORACLE])
        .args([target.tag.trim_start_matches('v'), &target.magic_hex()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the pinned CPython oracle");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let expected = child.wait_with_output().unwrap();
    assert!(
        expected.status.success(),
        "{version:?} {name}: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    let actual = compile_with_options(source, "private_type_parameters.py", &options)
        .unwrap_or_else(|error| panic!("{version:?} {name}: {error:?}"))
        .marshal();
    let first_difference = actual
        .iter()
        .zip(&expected.stdout)
        .position(|(a, b)| a != b);
    assert!(
        actual == expected.stdout,
        "{version:?} {name}: first difference {first_difference:?}, lengths {} and {}",
        actual.len(),
        expected.stdout.len()
    );
    actual
}

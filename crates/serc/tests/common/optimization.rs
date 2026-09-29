use std::ffi::OsString;
use std::io::Write as _;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use serc::{CompileOptions, OptimizationLevel, PythonVersion, compile_with_options};

pub(crate) const VERSIONS: [(PythonVersion, &str, &str); 4] = [
    (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
    (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
    (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
    (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
];
pub(crate) const LEVELS: [OptimizationLevel; 3] = [
    OptimizationLevel::Zero,
    OptimizationLevel::One,
    OptimizationLevel::Two,
];

pub(crate) static ORACLES: [OnceLock<OsString>; 4] = [const { OnceLock::new() }; 4];

const ORACLE_IDENTITY: &str = r"
import importlib.util, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
";
const MARSHAL_ORACLE: &str = r"
import marshal, sys
code = compile(sys.stdin.buffer.read(), sys.argv[2], 'exec', dont_inherit=True, optimize=int(sys.argv[1]))
sys.stdout.buffer.write(marshal.dumps(code))
";

pub(crate) fn oracle(
    source: &[u8],
    filename: &str,
    version: PythonVersion,
    level: OptimizationLevel,
) -> Output {
    let index = VERSIONS
        .iter()
        .position(|(candidate, _, _)| *candidate == version)
        .unwrap();
    let executable = ORACLES[index].get_or_init(|| {
        let (_, variable, fallback) = VERSIONS[index];
        let executable = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let target = version.target();
        let identity = Command::new(&executable)
            .args(["-I", "-S", "-c", ORACLE_IDENTITY])
            .args([target.tag.trim_start_matches('v'), &target.magic_hex()])
            .output()
            .expect("check the pinned CPython oracle");
        assert!(
            identity.status.success(),
            "{variable} must name {} with magic {}: {}",
            target.tag,
            target.magic_hex(),
            String::from_utf8_lossy(&identity.stderr)
        );
        executable
    });
    let level = match level {
        OptimizationLevel::Zero => "0",
        OptimizationLevel::One => "1",
        OptimizationLevel::Two => "2",
    };
    let mut child = Command::new(executable)
        .args(["-I", "-S", "-X", "utf8", "-c", MARSHAL_ORACLE])
        .args([level, filename])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the pinned CPython oracle");
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}

#[track_caller]
pub(crate) fn compile_both(
    source: &str,
    filename: &str,
    python_version: PythonVersion,
    optimization_level: OptimizationLevel,
) -> [Vec<u8>; 2] {
    let native = oracle(
        source.as_bytes(),
        filename,
        python_version,
        optimization_level,
    );
    assert!(
        native.status.success(),
        "{python_version}, {optimization_level:?}, {filename}: {}",
        String::from_utf8_lossy(&native.stderr)
    );
    let actual = compile_with_options(
        source,
        filename,
        &CompileOptions {
            python_version,
            optimization_level,
            ..CompileOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{python_version}, {optimization_level:?}, {filename}: {error}"))
    .marshal();
    [native.stdout, actual]
}

pub(crate) fn assert_matches_cpython(source: &str, filename: &str, levels: &[OptimizationLevel]) {
    let versions = VERSIONS.map(|(version, _, _)| version);
    assert_matches_cpython_versions(source, filename, levels, &versions);
}

pub(crate) fn assert_matches_cpython_versions(
    source: &str,
    filename: &str,
    levels: &[OptimizationLevel],
    versions: &[PythonVersion],
) {
    for &python_version in versions {
        for &optimization_level in levels {
            let [expected, actual] =
                compile_both(source, filename, python_version, optimization_level);
            let first_difference = actual
                .iter()
                .zip(&expected)
                .position(|(actual, expected)| actual != expected);
            assert!(
                actual == expected,
                "{python_version}, {optimization_level:?}, {filename}: first difference \
                 {first_difference:?}, lengths {} and {}",
                actual.len(),
                expected.len()
            );
        }
    }
}

/// Uses an oracle whose identity was checked by a preceding compilation.
pub(crate) fn probe(index: usize) -> Command {
    Command::new(ORACLES[index].get().expect("oracle identity was checked"))
}

/// Retains the command's stdout policy; callers pipe it when inspecting output.
#[track_caller]
pub(crate) fn run_successfully(command: &mut Command, input: &[u8]) -> Vec<u8> {
    let mut child = command
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}: status {}, {}",
        command.get_program().to_string_lossy(),
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}

#[track_caller]
pub(crate) fn assert_runs(command: &mut Command, [native, actual]: [&[u8]; 2]) {
    run_successfully(command, native);
    run_successfully(command, actual);
}

/// Captures stdout and compares it only after both runs succeed.
#[track_caller]
pub(crate) fn assert_outputs_match(command: &mut Command, [native, actual]: [&[u8]; 2]) -> Vec<u8> {
    command.stdout(Stdio::piped());
    let expected = run_successfully(command, native);
    let actual = run_successfully(command, actual);
    assert_eq!(
        actual,
        expected,
        "{}",
        command.get_program().to_string_lossy()
    );
    expected
}

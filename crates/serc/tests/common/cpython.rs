use std::ffi::{OsStr, OsString};
use std::io::Write as _;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use serc::PythonVersion;

pub(crate) const VERSIONS: [PythonVersion; 4] = [
    PythonVersion::Py312,
    PythonVersion::Py313,
    PythonVersion::Py314,
    PythonVersion::Py315,
];

pub(crate) const IDENTITY: &str = "import importlib.util, sys; expected = tuple(map(int, sys.argv[2].split('.'))) + (sys.argv[4], int(sys.argv[5])); actual = sys.version_info; magic = importlib.util.MAGIC_NUMBER.hex(); ok = sys.implementation.name == sys.argv[1] and actual == expected and sys.version.split()[0] == sys.argv[6] and magic == sys.argv[3]; print(f'{sys.implementation.name} {'.'.join(map(str, actual[:3]))} {actual.releaselevel} {actual.serial} magic {magic}', file=sys.stderr); raise SystemExit(not ok)";

static EXECUTABLES: [OnceLock<OsString>; 4] = [const { OnceLock::new() }; 4];

fn executable(version: PythonVersion) -> &'static OsStr {
    let index = VERSIONS
        .iter()
        .position(|&candidate| candidate == version)
        .unwrap();
    EXECUTABLES[index].get_or_init(|| {
        let (variable, fallback) = match version {
            PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12"),
            PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13"),
            PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
            PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
        };
        let executable = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let target = version.target();
        let (level, serial) = if version == PythonVersion::Py315 {
            ("candidate", "2")
        } else {
            ("final", "0")
        };
        let output = Command::new(&executable)
            .args([
                "-I",
                "-S",
                "-c",
                IDENTITY,
                target.implementation,
                &target.version_string(),
                &target.magic_hex(),
                level,
                serial,
                target.tag.trim_start_matches('v'),
            ])
            .output()
            .unwrap_or_else(|error| {
                panic!(
                    "failed to run {variable} ({}): {error}",
                    executable.display()
                )
            });
        assert!(
            output.status.success(),
            "{variable} must name CPython {} with magic {}: {}",
            target.tag,
            target.magic_hex(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        executable
    })
}

/// Validate identity separately so the caller's script controls the marshal process's state.
pub(crate) fn command(version: PythonVersion) -> Command {
    Command::new(executable(version))
}

pub(crate) fn run_with_input(command: &mut Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[allow(dead_code)]
pub(crate) mod diagnostics;

use std::process::Command;
use std::sync::OnceLock;

use serc::{CPYTHON_TARGET, PythonVersion};

#[allow(dead_code)]
pub(crate) const PYTHON_VERSIONS: [PythonVersion; 4] = [
    PythonVersion::Py312,
    PythonVersion::Py313,
    PythonVersion::Py314,
    PythonVersion::Py315,
];

/// Returns a command for the pinned interpreter.
#[allow(dead_code)]
pub(crate) fn python_command(version: PythonVersion) -> Command {
    match version {
        PythonVersion::Py312 => python_312_command(),
        PythonVersion::Py313 => python_313_command(),
        PythonVersion::Py314 => python_3145_command(),
        PythonVersion::Py315 => python_315_command(),
    }
}

const PYTHON_3145_IDENTITY_CHECK: &str = "import importlib.util, sys; expected = tuple(map(int, sys.argv[2].split('.'))) + ('final', 0); actual = sys.version_info; magic = importlib.util.MAGIC_NUMBER.hex(); ok = sys.implementation.name == sys.argv[1] and actual == expected and magic == sys.argv[3]; print(f'{sys.implementation.name} {'.'.join(map(str, actual[:3]))} {actual.releaselevel} {actual.serial} magic {magic}', file=sys.stderr); raise SystemExit(not ok)";

static PYTHON_3145: OnceLock<Result<String, String>> = OnceLock::new();

fn python_3145() -> &'static str {
    match PYTHON_3145.get_or_init(discover_python_3145) {
        Ok(executable) => executable,
        Err(error) => panic!("{error}"),
    }
}

fn discover_python_3145() -> Result<String, String> {
    let configured = std::env::var("SERC_TEST_PYTHON").ok();
    let executable = configured.as_deref().unwrap_or("python3.14");
    let version = CPYTHON_TARGET.version_string();
    let magic = CPYTHON_TARGET.magic_hex();
    let output = Command::new(executable)
        .args([
            "-I",
            "-S",
            "-c",
            PYTHON_3145_IDENTITY_CHECK,
            CPYTHON_TARGET.implementation,
            version.as_str(),
            magic.as_str(),
        ])
        .output()
        .map_err(|error| {
            format!(
                "oracle-dependent tests require CPython {version} with magic {magic}; failed to run `{executable}`: {error}. Set SERC_TEST_PYTHON to the pinned interpreter"
            )
        })?;
    if output.status.success() {
        return Ok(executable.to_string());
    }
    let source = if configured.is_some() {
        "SERC_TEST_PYTHON"
    } else {
        "the default `python3.14` interpreter"
    };
    Err(format!(
        "{source} must name CPython {version} with magic {magic}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

pub(crate) fn python_3145_command() -> Command {
    let mut command = Command::new(python_3145());
    command.args(["-I", "-S"]);
    command
}

static PYTHON_315: OnceLock<Result<String, String>> = OnceLock::new();

pub(crate) fn python_315_command() -> Command {
    let executable = PYTHON_315.get_or_init(|| {
        let target = serc::PythonVersion::Py315.target();
        let executable = std::env::var("SERC_TEST_PYTHON_315")
            .unwrap_or_else(|_| "python3.15".to_string());
        let output = Command::new(&executable)
            .args([
                "-B", "-I", "-S", "-c",
                "import importlib.util, sys; ok = sys.implementation.name == 'cpython' and sys.version_info == (3, 15, 0, 'candidate', 2) and sys.version.split()[0] == sys.argv[1] and importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; print(sys.version, file=sys.stderr); raise SystemExit(not ok)",
                target.tag.strip_prefix('v').expect("CPython release tag"),
                &target.magic_hex(),
            ])
            .output()
            .map_err(|error| format!("failed to run pinned CPython {} at {executable}: {error}; set SERC_TEST_PYTHON_315", target.tag))?;
        if !output.status.success() {
            return Err(format!("SERC_TEST_PYTHON_315 must name CPython {} with magic {}: {}", target.tag, target.magic_hex(), String::from_utf8_lossy(&output.stderr).trim()));
        }
        Ok(executable)
    });
    let mut command = Command::new(
        executable
            .as_ref()
            .unwrap_or_else(|error| panic!("{error}")),
    );
    command.args(["-B", "-I", "-S"]);
    command
}

static PYTHON_312: OnceLock<Result<String, String>> = OnceLock::new();
static PYTHON_313: OnceLock<Result<String, String>> = OnceLock::new();

pub(crate) fn python_312_command() -> Command {
    python_final_release_command(
        serc::PythonVersion::Py312,
        &PYTHON_312,
        "SERC_TEST_PYTHON_312",
        "python3.12",
    )
}

pub(crate) fn python_313_command() -> Command {
    python_final_release_command(
        serc::PythonVersion::Py313,
        &PYTHON_313,
        "SERC_TEST_PYTHON_313",
        "python3.13",
    )
}

fn python_final_release_command(
    version: serc::PythonVersion,
    cache: &'static OnceLock<Result<String, String>>,
    variable: &str,
    default: &str,
) -> Command {
    let executable = cache.get_or_init(|| {
        let target = version.target();
        let executable = std::env::var(variable).unwrap_or_else(|_| default.to_string());
        let output = Command::new(&executable)
            .args([
                "-B", "-I", "-S", "-c",
                "import importlib.util, sys; expected = tuple(map(int, sys.argv[2].split('.'))) + ('final', 0); ok = sys.implementation.name == sys.argv[1] and sys.version_info == expected and sys.version.split()[0] == sys.argv[2] and importlib.util.MAGIC_NUMBER.hex() == sys.argv[3]; print(sys.version, file=sys.stderr); raise SystemExit(not ok)",
                target.implementation,
                &target.version_string(),
                &target.magic_hex(),
            ])
            .output()
            .map_err(|error| format!("failed to run pinned CPython {} at {executable}: {error}; set {variable}", target.tag))?;
        if !output.status.success() {
            return Err(format!("{variable} must name CPython {} with magic {}: {}", target.tag, target.magic_hex(), String::from_utf8_lossy(&output.stderr).trim()));
        }
        Ok(executable)
    });
    let mut command = Command::new(
        executable
            .as_ref()
            .unwrap_or_else(|error| panic!("{error}")),
    );
    command.args(["-B", "-I", "-S"]);
    command
}

#[test]
fn python_3145_identity_rejects_prereleases() {
    let script = "import collections, sys; Version = collections.namedtuple('Version', 'major minor micro releaselevel serial'); sys.version_info = Version(3, 14, 5, 'candidate', 1); exec(sys.argv.pop(1))";
    let output = Command::new(python_3145())
        .args([
            "-I",
            "-S",
            "-c",
            script,
            PYTHON_3145_IDENTITY_CHECK,
            CPYTHON_TARGET.implementation,
            &CPYTHON_TARGET.version_string(),
            &CPYTHON_TARGET.magic_hex(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        format!(
            "cpython 3.14.5 candidate 1 magic {}",
            CPYTHON_TARGET.magic_hex()
        )
    );
}

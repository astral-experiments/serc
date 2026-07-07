use std::process::Command;
use std::sync::OnceLock;

use serc::CPYTHON_TARGET;

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
            "import importlib.util, sys; expected = tuple(map(int, sys.argv[2].split('.'))); actual = sys.version_info[:3]; magic = importlib.util.MAGIC_NUMBER.hex(); ok = sys.implementation.name == sys.argv[1] and actual == expected and magic == sys.argv[3]; print(f'{sys.implementation.name} {'.'.join(map(str, actual))} magic {magic}', file=sys.stderr); raise SystemExit(not ok)",
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

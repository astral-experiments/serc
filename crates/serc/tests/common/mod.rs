pub(crate) mod cpython;

use std::process::Command;

use serc::{compile, compile_bytes, compile_to_pyc};
use tempfile::tempdir;

const CPYTHON_MARSHAL: &str = "import marshal, sys; code = compile(sys.stdin.read(), sys.argv[1], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
const CPYTHON_MARSHAL_BYTES: &str = "import marshal, sys; code = compile(sys.stdin.buffer.read(), sys.argv[1], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
const CPYTHON_POP_EXCEPT_POSITIONS: &str = r#"
import dis, marshal, sys, types
if sys.argv[1] == "source":
    root = compile(sys.stdin.buffer.read(), sys.argv[2], "exec", dont_inherit=True, optimize=0)
else:
    root = marshal.loads(sys.stdin.buffer.read())

def walk(code):
    yield code
    for value in code.co_consts:
        if isinstance(value, types.CodeType):
            yield from walk(value)

target = sys.argv[3:]
for code in walk(root):
    positions = [
        (
            instruction.positions.lineno,
            instruction.positions.end_lineno,
            instruction.positions.col_offset,
            instruction.positions.end_col_offset,
        )
        for instruction in dis.get_instructions(code, show_caches=True)
        if instruction.opname == "POP_EXCEPT"
    ]
    if target:
        if code.co_qualname == target[0]:
            print(any(position[0] == int(target[1]) for position in positions))
    elif positions:
        print(code.co_qualname, positions)
"#;

pub(crate) fn python_3145_command() -> Command {
    let mut command = cpython::command(serc::PythonVersion::Py314);
    // Keep text passed through stdin and stdout deterministic across the Unix UTF-8 locale and
    // Windows' legacy console code page. `-I` ignores `PYTHONUTF8`, so select UTF-8 explicitly.
    command.args(["-I", "-S", "-X", "utf8"]);
    command
}

fn python_3145_output(arguments: &[&str], input: &[u8]) -> std::process::Output {
    cpython::run_with_input(python_3145_command().args(arguments), input)
}

fn python_text_stdout(stdout: Vec<u8>) -> String {
    String::from_utf8(stdout)
        .expect("CPython text oracle should emit UTF-8")
        .replace("\r\n", "\n")
}

#[test]
fn python_text_stdout_normalizes_windows_newlines() {
    assert_eq!(
        python_text_stdout(b"first\r\nsecond\r\n".to_vec()),
        "first\nsecond\n"
    );
}

fn cpython_pop_except_positions(input: &[u8], mode: &str, filename: &str) -> std::process::Output {
    python_3145_output(&["-c", CPYTHON_POP_EXCEPT_POSITIONS, mode, filename], input)
}

pub(crate) fn assert_matches_cpython_pop_except_positions(source: &str, filename: &str) {
    let expected = cpython_pop_except_positions(source.as_bytes(), "source", filename);
    assert!(
        expected.status.success(),
        "CPython POP_EXCEPT position oracle failed for {filename}: {}",
        String::from_utf8_lossy(&expected.stderr)
    );

    let marshal = compile(source, filename).unwrap().marshal();
    let actual = cpython_pop_except_positions(&marshal, "marshal", filename);
    assert!(
        actual.status.success(),
        "CPython failed to decode Serc marshal for {filename}: {}",
        String::from_utf8_lossy(&actual.stderr)
    );

    assert_eq!(
        actual.stdout, expected.stdout,
        "POP_EXCEPT positions differ for {filename}",
    );
}
fn cpython_has_pop_except_at(
    input: &[u8],
    mode: &str,
    filename: &str,
    qualified_name: &str,
    line: u32,
) -> std::process::Output {
    let line = line.to_string();
    python_3145_output(
        &[
            "-c",
            CPYTHON_POP_EXCEPT_POSITIONS,
            mode,
            filename,
            qualified_name,
            line.as_str(),
        ],
        input,
    )
}

pub(crate) fn assert_cpython_and_serc_have_pop_except_at(
    source: &str,
    filename: &str,
    qualified_name: &str,
    line: u32,
) {
    let expected =
        cpython_has_pop_except_at(source.as_bytes(), "source", filename, qualified_name, line);
    assert!(
        expected.status.success(),
        "CPython POP_EXCEPT position oracle failed for {filename}: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(
        python_text_stdout(expected.stdout.clone()),
        "True\n",
        "CPython control shape changed"
    );

    let marshal = compile(source, filename).unwrap().marshal();
    let actual = cpython_has_pop_except_at(&marshal, "marshal", filename, qualified_name, line);
    assert!(
        actual.status.success(),
        "CPython failed to decode Serc marshal for {filename}: {}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout, expected.stdout,
        "Serc lost the located POP_EXCEPT in {qualified_name}",
    );
}

pub(crate) fn assert_matches_cpython_marshal(source: &str, filename: &str) {
    let expected = python_3145_output(&["-c", CPYTHON_MARSHAL, filename], source.as_bytes());
    assert!(
        expected.status.success(),
        "CPython marshal oracle failed for {filename}: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(
        compile(source, filename).unwrap().marshal(),
        expected.stdout,
        "marshal mismatch for {filename}",
    );
}

pub(crate) fn assert_matches_cpython_startup_marshal(source: &str, filename: &str) {
    #[cfg(target_os = "linux")]
    startup_reference::assert_matches(source, filename);
    #[cfg(not(target_os = "linux"))]
    assert_matches_cpython_marshal(source, filename);
}

#[cfg(target_os = "linux")]
mod startup_reference {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::OnceLock;

    use serc::{CPYTHON_TARGET, compile};

    static REFERENCE: OnceLock<Result<PathBuf, String>> = OnceLock::new();

    fn command(executable: &Path) -> Command {
        let mut command = Command::new(executable);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8");
        command
    }

    fn expected_identity() -> serde_json::Value {
        serde_json::json!({
            "protocol": "minimal-v2",
            "implementation": CPYTHON_TARGET.implementation,
            "version": CPYTHON_TARGET.version_string(),
            "release_level": "final",
            "release_serial": 0,
            "magic": CPYTHON_TARGET.magic_hex(),
        })
    }

    fn discover(configured: Option<&OsStr>) -> Result<PathBuf, String> {
        let configured = configured.ok_or_else(|| {
            "startup-sensitive tests require SERC_TEST_CPYTHON_REFERENCE; build it with scripts/build-cpython-reference.py".to_string()
        })?;
        let executable = Path::new(configured)
            .canonicalize()
            .map_err(|error| format!("invalid SERC_TEST_CPYTHON_REFERENCE: {error}"))?;
        let output = command(&executable)
            .arg("--identity")
            .output()
            .map_err(|error| format!("failed to run CPython reference identity: {error}"))?;
        if !output.status.success() {
            return Err(format!("CPython reference identity failed: {output:?}"));
        }
        let actual: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("invalid CPython reference identity: {error}"))?;
        if actual != expected_identity() {
            return Err(format!("unexpected CPython reference identity: {actual}"));
        }
        Ok(executable)
    }

    pub(super) fn assert_matches(source: &str, filename: &str) {
        let executable = REFERENCE
            .get_or_init(|| discover(std::env::var_os("SERC_TEST_CPYTHON_REFERENCE").as_deref()))
            .as_ref()
            .unwrap_or_else(|error| panic!("{error}"));
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(Path::new(filename).file_name(), Some(OsStr::new(filename)));
        fs_err::write(directory.path().join(filename), source).unwrap();
        // The native startup profile excludes Python wrapper imports. Keep the
        // original relative co_filename while the native helper reads its input.
        let expected = command(executable)
            .current_dir(directory.path())
            .args(["--optimize", "0", filename])
            .output()
            .unwrap();
        assert!(
            expected.status.success(),
            "native oracle failed: {expected:?}"
        );
        assert_eq!(
            compile(source, filename).unwrap().marshal(),
            expected.stdout,
            "native startup marshal mismatch for {filename}",
        );
    }

    #[test]
    fn missing_reference_is_an_error() {
        assert!(
            discover(None)
                .unwrap_err()
                .contains("SERC_TEST_CPYTHON_REFERENCE")
        );
        let directory = tempfile::tempdir().unwrap();
        assert!(discover(Some(directory.path().join("missing").as_os_str())).is_err());
    }

    #[test]
    fn wrong_reference_identity_is_an_error() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reference");
        let write_identity = |identity: &serde_json::Value| {
            fs_err::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{identity}'\n")).unwrap();
            fs_err::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        write_identity(&expected_identity());
        assert_eq!(discover(Some(path.as_os_str())).unwrap(), path);
        for (field, value) in [
            ("protocol", serde_json::json!("other")),
            ("protocol", serde_json::json!("minimal-v1")),
            ("implementation", serde_json::json!("other")),
            ("version", serde_json::json!("3.14.4")),
            ("release_level", serde_json::json!("candidate")),
            ("release_serial", serde_json::json!(1)),
            ("magic", serde_json::json!("00000000")),
        ] {
            let mut identity = expected_identity();
            identity[field] = value;
            write_identity(&identity);
            assert!(discover(Some(path.as_os_str())).is_err(), "{field}");
        }
    }
}

pub(crate) fn assert_matches_cpython_marshal_bytes(source: &[u8], filename: &str) {
    let expected = python_3145_output(&["-c", CPYTHON_MARSHAL_BYTES, filename], source);
    assert!(
        expected.status.success(),
        "CPython byte-source marshal oracle failed for {filename}: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(
        compile_bytes(source, filename).unwrap().marshal(),
        expected.stdout,
        "marshal mismatch for byte source {filename}",
    );
}

pub(crate) fn assert_executes(source: &str, expected: &str) {
    let temporary = tempdir().unwrap();
    let pyc_path = temporary.path().join("program.pyc");
    let pyc = compile_to_pyc(source, "program.py", 0).unwrap();
    fs_err::write(&pyc_path, pyc).unwrap();

    let output = python_3145_command().arg(&pyc_path).output().unwrap();
    assert!(
        output.status.success(),
        "generated bytecode failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(python_text_stdout(output.stdout), expected);
}

pub(crate) fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

#[test]
fn python_3145_identity_rejects_prereleases() {
    use serc::CPYTHON_TARGET;

    let script = "import collections, sys; Version = collections.namedtuple('Version', 'major minor micro releaselevel serial'); sys.version_info = Version(3, 14, 5, 'candidate', 1); exec(sys.argv.pop(1))";
    let output = cpython::command(serc::PythonVersion::Py314)
        .args([
            "-I",
            "-S",
            "-c",
            script,
            cpython::IDENTITY,
            CPYTHON_TARGET.implementation,
            &CPYTHON_TARGET.version_string(),
            &CPYTHON_TARGET.magic_hex(),
            "final",
            "0",
            CPYTHON_TARGET.tag.trim_start_matches('v'),
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

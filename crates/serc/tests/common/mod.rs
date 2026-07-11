use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use serc::{CPYTHON_TARGET, compile, compile_bytes, compile_to_pyc};
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

fn cpython_pop_except_positions(input: &[u8], mode: &str, filename: &str) -> std::process::Output {
    python_3145_command()
        .args(["-c", CPYTHON_POP_EXCEPT_POSITIONS, mode, filename])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input)?;
            child.wait_with_output()
        })
        .unwrap()
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
    python_3145_command()
        .args([
            "-c",
            CPYTHON_POP_EXCEPT_POSITIONS,
            mode,
            filename,
            qualified_name,
            line.as_str(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input)?;
            child.wait_with_output()
        })
        .unwrap()
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
    assert_eq!(expected.stdout, b"True\n", "CPython control shape changed");

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
    let expected = python_3145_command()
        .args(["-c", CPYTHON_MARSHAL, filename])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(source.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
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

pub(crate) fn assert_matches_cpython_marshal_bytes(source: &[u8], filename: &str) {
    let expected = python_3145_command()
        .args(["-c", CPYTHON_MARSHAL_BYTES, filename])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(source)?;
            child.wait_with_output()
        })
        .unwrap();
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
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

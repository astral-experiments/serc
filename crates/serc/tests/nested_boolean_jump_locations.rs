use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

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
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "nested_boolean_jump_locations.py";
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
        assert_eq!(actual.marshal(), expected.stdout, "{version:?}: {source}");
    }
}

#[test]
fn threaded_boolean_jumps_keep_intermediate_locations_and_ownerless_slots() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for mask in 0..16 {
            let [outer, middle, inner, leaf] =
                [0, 1, 2, 3].map(|bit| if mask & (1 << bit) == 0 { "and" } else { "or" });
            for operand in ["e", "not e", "e is None", "e not in {'first', 'second'}"] {
                let expression = format!(
                    "(\n        a\n        {outer} (\n            b\n            {middle} (\n                (\n                    c\n                    {inner} (not d {leaf} {operand})\n                )\n                {outer} f\n            )\n        )\n        {outer} g\n    )"
                );
                let indented = expression.replace('\n', "\n    ");
                for source in [
                    format!("def f(a,b,c,d,e,f,g):\n    return {expression}\n"),
                    format!(
                        "def f(values):\n    return any({}\n        for a,b,c,d,e,f,g in values\n    ))\n",
                        expression.strip_suffix(')').unwrap()
                    ),
                    format!(
                        "def f(a,b,c,d,e,f,g):\n    try:\n        return {indented}\n    except Error:\n        return None\n"
                    ),
                    format!(
                        "def f(a,b,c,d,e,f,g):\n    with manager():\n        return {indented}\n"
                    ),
                ] {
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

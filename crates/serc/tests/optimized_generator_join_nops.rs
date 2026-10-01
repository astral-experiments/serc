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
    let filename = "optimized_generator_join_nops.py";
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
        "CPython oracle failed: {}",
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
fn optimized_generator_join_keeps_its_nop_before_small_call_raise_tails() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for definition in ["def", "async def"] {
            for callee in ["tuple", "list", "set", "custom"] {
                for keyword in [false, true] {
                    for trailing in 0..=2 {
                        let expression =
                            format!("{callee}(item.value for item in items) if flag else None");
                        let first = if keyword {
                            format!("result={expression}")
                        } else {
                            expression
                        };
                        let mut arguments = vec![first];
                        for index in 0..trailing {
                            arguments.push(if keyword {
                                format!("tail{index}=flag")
                            } else {
                                "flag".into()
                            });
                        }
                        let source = format!(
                            "{definition} f(flag, items):\n    raise outer(\n        {},\n    )\n",
                            arguments.join(",\n        "),
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn generator_join_same_line_and_nonoptimized_controls_stay_exact() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for source in [
            "def f(flag, items):\n    raise outer(tuple(item.value for item in items) if flag else None)\n",
            "def f(flag, items):\n    raise outer(result=tuple(item.value for item in items) if flag else None, tail=flag)\n",
            "def f(flag, items):\n    raise outer(\n        custom(item.value for item in items) if flag else None,\n    )\n",
            "def f(flag, items):\n    raise outer(\n        None if flag else tuple(item.value for item in items),\n    )\n",
            "def f(flag, items):\n    return outer(\n        tuple(item.value for item in items) if flag else None,\n    )\n",
            "def f(flag, items):\n    raise outer(\n        result=tuple(item.value for item in items) if flag else None,\n        tail=flag,\n        other=flag,\n    )\n",
        ] {
            assert_matches_cpython(source, version);
        }
    }
}

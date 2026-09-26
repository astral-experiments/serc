mod common;

use std::process::Command;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[test]
fn bare_yield_commas_match_native_grammar() {
    let cases = [
        ("def f():\n    yield, x\n", Some("invalid syntax")),
        ("def f():\n    x = yield, y\n", Some("invalid syntax")),
        ("def f():\n    (yield, x)\n", Some("invalid syntax")),
        ("def f():\n    (yield,)\n", Some("invalid syntax")),
        ("def f():\n    yield \\\n    , x\n", Some("invalid syntax")),
        (
            "def f():\n    (yield # comment\n     , x)\n",
            Some("invalid syntax"),
        ),
        (
            "def f():\n    if False:\n        yield, x\n",
            Some("invalid syntax"),
        ),
        (
            "def f():\n    return\n    yield, x\n",
            Some("invalid syntax"),
        ),
        (
            "from __future__ import annotations\nx: (yield, 1)\n",
            Some("invalid syntax"),
        ),
        ("def f():\n    (yield), x\n", None),
        ("def f():\n    yield\n", None),
        ("def f():\n    (yield)\n", None),
        ("def f():\n    yield x, y\n", None),
        ("def f():\n    yield (x, y)\n", None),
        ("def f():\n    ((yield), x)\n", None),
        ("def f():\n    yield # , x\n", None),
        ("def f():\n    yield \"yield, x\"\n", None),
        ("text = f'yield, {value}'\n", None),
        ("def f():\n    (yield # comment\n    ), x\n", None),
    ];
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bare_yield.py");
    let oracle = r"
import marshal, sys
try:
    code = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
    print(error.msg)
    raise SystemExit(1)
sys.stdout.buffer.write(marshal.dumps(code))
";
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (source, expected) in cases {
            let mut python = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = python
                .args(["-c", oracle, source])
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(
                native.status.success(),
                expected.is_none(),
                "{version}: {source}: {native:?}"
            );
            for compiled in [
                compile_with_options(source, path.to_str().unwrap(), &options),
                compile_bytes_with_options(source.as_bytes(), path.to_str().unwrap(), &options),
            ] {
                if let Some(expected) = expected {
                    assert_eq!(String::from_utf8_lossy(&native.stdout).trim(), expected);
                    let error = compiled.unwrap_err();
                    assert!(
                        error.to_string().ends_with(expected),
                        "{version}: {source}: {error}"
                    );
                } else {
                    assert_eq!(
                        compiled.unwrap().marshal(),
                        native.stdout,
                        "{version}: {source}"
                    );
                }
            }
            fs_err::write(&path, source).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_serc"))
                .args([
                    "compile",
                    "--check",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                ])
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(
                output.status.success(),
                expected.is_none(),
                "{version}: {source}: {output:?}"
            );
            if let Some(expected) = expected {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains(expected),
                    "{output:?}"
                );
            }
        }
    }
}

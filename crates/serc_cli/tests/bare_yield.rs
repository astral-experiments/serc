mod common;

use std::process::Command;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[test]
fn bare_yield_commas_match_native_grammar() {
    const F_DELIMITER: &str = "f-string: expecting '=', or '!', or ':', or '}'";
    const F_EXPRESSION: &str = "f-string: expecting a valid expression after '{'";
    const T_DELIMITER: &str = "t-string: expecting '=', or '!', or ':', or '}'";
    const T_EXPRESSION: &str = "t-string: expecting a valid expression after '{'";
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
        ("def f():\n    f'{yield, 1}'\n", Some(F_DELIMITER)),
        ("def f():\n    f'{yield,}'\n", Some(F_DELIMITER)),
        (
            "def f():\n    f'{yield # comment\n    , 1}'\n",
            Some(F_DELIMITER),
        ),
        ("def f():\n    f'{yield \\\n    , 1}'\n", Some(F_DELIMITER)),
        ("def f():\n    f'{(yield, 1)}'\n", Some(F_EXPRESSION)),
        ("def f():\n    f'{((yield, 1))}'\n", Some(F_EXPRESSION)),
        ("def f():\n    f'{[1, (yield, 1)]}'\n", Some(F_EXPRESSION)),
        ("def f():\n    f'{not (yield, 1)}'\n", Some(F_EXPRESSION)),
        (
            "def f():\n    f'{(lambda: (yield, 1))}'\n",
            Some(F_EXPRESSION),
        ),
        ("def f():\n    f'{1 + (yield, 1)}'\n", Some(F_DELIMITER)),
        ("def f():\n    f'{f((yield, 1))}'\n", Some(F_DELIMITER)),
        ("def f():\n    f'{yield (yield, 1)}'\n", Some(F_DELIMITER)),
        ("def f():\n    f'{1, (yield, 1)}'\n", Some(F_DELIMITER)),
        ("def f():\n    f'{1:{yield, 1}}'\n", Some(F_DELIMITER)),
        ("def f():\n    f'{1:{(yield, 1)}}'\n", Some(F_EXPRESSION)),
        ("def f():\n    f\"{f'{yield, 1}'}\"\n", Some(F_DELIMITER)),
        ("def f():\n    f\"{f'{(yield, 1)}'}\"\n", Some(F_EXPRESSION)),
        ("def f():\n    f'{yield}'\n", None),
        ("def f():\n    f'{(yield), 1}'\n", None),
        ("def f():\n    f'{1:{(yield), 1}}'\n", None),
        ("def f():\n    f'{yield:,}'\n", None),
        ("def f():\n    f'{{yield, 1}}'\n", None),
        (
            "f(x=1, x=2)\ndef g():\n    yield, 1\n",
            Some("invalid syntax"),
        ),
        (
            "def g():\n    yield, 1\nf(x=1, x=2)\n",
            Some("invalid syntax"),
        ),
        (
            "obj.__debug__ = 1\ndef g():\n    yield, 1\n",
            Some("invalid syntax"),
        ),
        (
            "def g():\n    yield, 1\nobj.__debug__ = 1\n",
            Some("invalid syntax"),
        ),
    ];
    let template_cases = [
        ("def f():\n    t'{yield, 1}'\n", Some(T_DELIMITER)),
        ("def f():\n    t'{(yield, 1)}'\n", Some(T_EXPRESSION)),
        ("def f():\n    t'{1 + (yield, 1)}'\n", Some(T_DELIMITER)),
        ("def f():\n    t'{1:{yield, 1}}'\n", Some(T_DELIMITER)),
        ("def f():\n    t'{1:{(yield, 1)}}'\n", Some(T_EXPRESSION)),
        ("def f():\n    t\"{f'{yield, 1}'}\"\n", Some(F_DELIMITER)),
        ("def f():\n    f\"{t'{yield, 1}'}\"\n", Some(T_DELIMITER)),
        ("def f():\n    t'{yield}'\n", None),
        ("def f():\n    t'{(yield), 1}'\n", None),
        ("def f():\n    t'yield, {value}'\n", None),
    ];
    let legacy_template_cases = [
        ("def f():\n    t'{yield, 1}'\n", Some("invalid syntax")),
        ("def f():\n    f\"{t'{yield, 1}'}\"\n", Some(F_DELIMITER)),
        ("def f():\n    f\"{tr'{yield, 1}'}\"\n", Some(F_DELIMITER)),
        ("def f():\n    f\"{(t'{yield, 1}')}\"\n", Some(F_EXPRESSION)),
        (
            "def f():\n    f\"{yield t'{yield, 1}'}\"\n",
            Some(F_DELIMITER),
        ),
        (
            "def f():\n    f\"{yield from t'{yield, 1}'}\"\n",
            Some(F_DELIMITER),
        ),
        (
            "def f():\n    f\"{type + t'{yield, 1}'}\"\n",
            Some("invalid syntax. Perhaps you forgot a comma?"),
        ),
        (
            "def f():\n    f\"{name + t'{yield, 1}'}\"\n",
            Some("invalid syntax. Perhaps you forgot a comma?"),
        ),
        (
            "def f():\n    f\"{(1 if c else 2) + t'{yield, 1}'}\"\n",
            Some("invalid syntax. Perhaps you forgot a comma?"),
        ),
        (
            "def f():\n    f\"{(lambda: 1) + t'{yield, 1}'}\"\n",
            Some("invalid syntax. Perhaps you forgot a comma?"),
        ),
        (
            "def f():\n    f\"{(1, 2) + t'{yield, 1}'}\"\n",
            Some("invalid syntax. Perhaps you forgot a comma?"),
        ),
        (
            "def f():\n    f\"{1 + t'{yield, 1}'}\"\n",
            Some("invalid syntax. Perhaps you forgot a comma?"),
        ),
        (
            "def f():\n    f\"{(1 + t'{yield, 1}')}\"\n",
            Some("invalid syntax. Perhaps you forgot a comma?"),
        ),
        (
            "def f():\n    t\"{f'{yield, 1}'}\"\n",
            Some("invalid syntax"),
        ),
    ];
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bare_yield.py");
    let oracle = r"
import marshal, sys
try:
    code = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
    print(error.msg)
    print(error.lineno, error.offset)
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
        for (source, expected) in cases
            .into_iter()
            .chain(
                template_cases
                    .into_iter()
                    .filter(|_| version >= PythonVersion::Py314),
            )
            .chain(
                legacy_template_cases
                    .into_iter()
                    .filter(|_| version < PythonVersion::Py314),
            )
        {
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
                    let native_error = String::from_utf8_lossy(&native.stdout);
                    let mut lines = native_error.lines();
                    assert_eq!(lines.next().unwrap(), expected);
                    let position: Vec<usize> = lines
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .map(|value| value.parse().unwrap())
                        .collect();
                    let expected_start = source
                        .split_inclusive('\n')
                        .take(position[0] - 1)
                        .map(str::len)
                        .sum::<usize>()
                        + position[1]
                        - 1;
                    let error = compiled.unwrap_err();
                    assert!(
                        error.to_string().ends_with(expected),
                        "{version}: {source}: {error}"
                    );
                    if expected.starts_with("f-string:")
                        || expected.starts_with("t-string:")
                        || expected.ends_with("Perhaps you forgot a comma?")
                    {
                        assert_eq!(
                            error.range().unwrap().start().to_usize(),
                            expected_start,
                            "{version}: {source}: {error}"
                        );
                    }
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

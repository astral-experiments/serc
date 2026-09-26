mod common;

use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    let prefix: usize = source
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    let line = source[prefix..].split('\n').next().unwrap();
    prefix
        + line
            .char_indices()
            .nth(column - 1)
            .map_or(line.len(), |(offset, _)| offset)
}

#[test]
fn invalid_alias_defaults_match_native_diagnostics() {
    let yield_statement = "def f():\n    yield, x\n";
    let mut aliases: Vec<String> = [
        "type X[T = int] = T\n",
        "type X[T, U = list[int] | tuple[str]] = U\n",
        "type X[*Ts, U = int] = U\n",
        "type X[T = (int or str)] = T\n",
        "type X[T = int or str] = T\n",
        "type X[T = (lambda: int)] = T\n",
        "type X[T = (int, str)] = T\n",
        "type X[\n  T = # comment\n  int\n] = T\n",
        "type X[T = f'{int}'] = T\n",
        "type X[\n  T = (\n    f'{int}'\n  )\n] = T\n",
        "type X[\n  Σ = (\n        f'{α}'\n  )\n] = Σ\n",
        "type X[\n  T = (\n    α | f'{β}'\n  )\n] = T\n",
        "type X[\n  T = (\n    int\n  )\n] = T\n",
        "type X[\r\n  T = (\r\n    f'{int}'\r\n  )\r\n] = T\r\n",
        "type X[\n  T = (\n甲乙丙\n  )\n] = T\n",
        "type X[\n  ΣΣΣΣΣ = (\n    α\n  )\n] = ΣΣΣΣΣ\n",
        "type X[Σ = α] = Σ\n",
        "if True:\n    type X[T = int] = T\n",
        "x = 1; type X[T = int] = T\n",
        "type X[**P] = int\ntype Y[U = int] = U\n",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    aliases.push(format!("type X[T = {}int] = T\n", "+".repeat(300)));
    aliases.push(format!(
        "type X[T = {}int{}] = T\n",
        "(".repeat(180),
        ")".repeat(180)
    ));
    let oracle = r"
import sys
diagnostic = None
for source in (sys.argv[1], sys.argv[1].encode()):
    try:
        compile(source, 'case.py', 'exec', dont_inherit=True)
    except SyntaxError as error:
        actual = (error.msg, error.lineno, error.offset, error.end_lineno, error.end_offset)
        assert diagnostic is None or diagnostic == actual
        diagnostic = actual
    else:
        raise AssertionError('expected a syntax error')
print(diagnostic[0])
print(*diagnostic[1:])
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
        for alias in &aliases {
            for suffix in [yield_statement, "@\n"] {
                let source = format!("{alias}{suffix}");
                let mut python = match version {
                    PythonVersion::Py312 => common::python_312_command(),
                    PythonVersion::Py313 => common::python_313_command(),
                    PythonVersion::Py314 => common::python_3145_command(),
                    PythonVersion::Py315 => common::python_315_command(),
                };
                let output = python.args(["-c", oracle, &source]).output().unwrap();
                assert!(output.status.success(), "{version}: {source}: {output:?}");
                let output = String::from_utf8(output.stdout).unwrap();
                let mut lines = output.lines();
                let message = lines.next().unwrap();
                let positions: Vec<usize> = lines
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .map(|value| value.parse().unwrap())
                    .collect();
                let normalized = source.replace("\r\n", "\n");
                let expected = (
                    byte_offset(&normalized, positions[0], positions[1]),
                    byte_offset(&normalized, positions[2], positions[3]),
                );
                for result in [
                    compile_with_options(&source, "case.py", &options),
                    compile_bytes_with_options(source.as_bytes(), "case.py", &options),
                ] {
                    let error = result.unwrap_err();
                    assert!(
                        error.to_string().ends_with(message),
                        "{version}: {source:?}: {message:?} != {error}"
                    );
                    let range = error.range().unwrap();
                    assert_eq!(
                        (range.start().to_usize(), range.end().to_usize()),
                        expected,
                        "{version}: {source:?}: {error}"
                    );
                }
            }
        }
    }
}

#[test]
fn alias_replay_preserves_valid_and_prior_errors() {
    let alias = "type X[T = int] = T\n";
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
        if version >= PythonVersion::Py313 {
            assert!(
                compile_with_options(alias, "case.py", &options).is_ok(),
                "{version}"
            );
        }
        for source in [
            format!("@\n{alias}"),
            format!("{alias}def f():\n    yield, x\nx = 'unterminated\n"),
            format!("{alias}def f():\n    yield, x\n)\n"),
        ] {
            let error = compile_with_options(&source, "case.py", &options).unwrap_err();
            assert!(
                matches!(error, CompileError::Parse(_)),
                "{version}: {source:?}: {error}"
            );
        }
        if version == PythonVersion::Py312 {
            // A parameter specification default in an earlier alias takes
            // precedence over replaying the later ordinary default.
            let source = "type X[**P = ...] = int\ntype Y[U = int] = U\n@\n";
            for result in [
                compile_with_options(source, "case.py", &options),
                compile_bytes_with_options(source.as_bytes(), "case.py", &options),
            ] {
                let error = result.unwrap_err();
                assert!(
                    matches!(error, CompileError::Parse(_)),
                    "{version}: {error}"
                );
            }
        }
        // A bound or an unparenthesized lambda does not take the assignment alternative.
        if version >= PythonVersion::Py313 {
            for alias in [
                "type X[T: int = str] = T\n",
                "type X[T = lambda: int] = T\n",
                "type X[T: int = str, U = int] = T\n",
                "type X[T = lambda: int, U = int] = T\n",
                "type X[T = not int, U = int] = T\n",
                "type X[**P, U = int] = T\n",
            ] {
                let source = format!("{alias}def f():\n    yield, x\n");
                let error = compile_with_options(&source, "case.py", &options).unwrap_err();
                assert!(
                    error.to_string().ends_with("invalid syntax"),
                    "{version}: {source:?}: {error}"
                );
                assert_eq!(
                    error.range().unwrap().start().to_usize(),
                    source.rfind(", x").unwrap()
                );
            }
            // A preceding starred default has its own native diagnostic. Until
            // that alternative is modeled, retain the earlier baseline error.
            let source = "type X[*Ts = tuple[int], U = int] = T\ndef f():\n    yield, x\n";
            let error = compile_with_options(source, "case.py", &options).unwrap_err();
            assert!(
                error.to_string().ends_with("invalid syntax"),
                "{version}: {error}"
            );
            assert_eq!(
                error.range().unwrap().start().to_usize(),
                source.find(", x").unwrap()
            );
        }
    }
}

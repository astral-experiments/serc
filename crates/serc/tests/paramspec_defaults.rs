use std::ffi::OsString;
use std::io::Write;
use std::process::{Command, Stdio};

use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

#[test]
fn python_312_rejects_parameter_specification_defaults() {
    let cases = [
        ("type Alias[**P = ...] = int\n", "invalid syntax", "="),
        ("class C[**P = ...]: pass\n", "invalid syntax", "="),
        ("def f[**P = ...](): pass\n", "expected '('", "["),
        ("async def f[**P = ...](): pass\n", "expected '('", "["),
        ("type Café[**P = ...] = int\n", "invalid syntax", "="),
        ("type Alias[**match = ...] = int\n", "invalid syntax", "="),
        (
            "type Alias[\n    **P  # comment\n    = ...\n] = int\n",
            "invalid syntax",
            "=",
        ),
        (
            "if False:\n    type Alias[**P = ...] = int\n",
            "invalid syntax",
            "=",
        ),
    ];
    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    for (source, expected_message, highlighted) in cases {
        for result in [
            compile_with_options(source, "paramspec.py", &options),
            compile_bytes_with_options(source.as_bytes(), "paramspec.py", &options),
        ] {
            let CompileError::InvalidSyntax { message, range } = result.unwrap_err() else {
                panic!("expected a syntax diagnostic for {source:?}");
            };
            assert_eq!(message, expected_message, "{source}");
            assert_eq!(&source[range], highlighted, "{source}");
            assert_eq!(
                usize::from(range.start()),
                source.find(highlighted).unwrap()
            );
        }
    }
}

#[test]
fn existing_parse_and_earlier_unsupported_errors_keep_their_priority() {
    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    let lexical_error = "type Alias[**P = ...] = int\nx = 'unterminated\n";
    assert!(matches!(
        compile_with_options(lexical_error, "paramspec.py", &options),
        Err(CompileError::Parse(_))
    ));
    let earlier_default = "type Alias[T = int, **P = ...] = int\n";
    let error = compile_with_options(earlier_default, "paramspec.py", &options).unwrap_err();
    // Python 3.12 reports the first default as an invalid assignment before
    // reaching the unsupported ParamSpec default.
    let CompileError::InvalidSyntax { message, range } = error else {
        panic!("expected an invalid syntax diagnostic for {earlier_default:?}");
    };
    assert_eq!(
        message,
        "invalid syntax. Maybe you meant '==' or ':=' instead of '='?"
    );
    assert_eq!(&earlier_default[range], "T = int");
    assert_eq!(usize::from(range.start()), 11);
    assert_eq!(usize::from(range.end()), 18);
}

#[test]
fn python_312_still_accepts_parameter_specifications_without_defaults() {
    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    for source in [
        "type Alias[**P] = int\n",
        "class C[**P]: pass\n",
        "def f[**P](): pass\n",
        "value = '**P = ...'\n",
    ] {
        compile_with_options(source, "paramspec.py", &options).unwrap();
    }
}

#[test]
fn newer_targets_keep_parameter_specification_defaults() {
    const ORACLE: &str = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), 'paramspec.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    let cases = [
        "type Alias[**P = ...] = int\n",
        "class C[**P = ...]: pass\n",
        "def f[**P = ...](): pass\n",
        "async def f[**P = ...](): pass\n",
    ];
    for (version, variable, fallback) in [
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let executable: OsString = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let target = version.target();
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in cases {
            let mut child = Command::new(&executable)
                .args(["-I", "-S", "-c", ORACLE])
                .args([target.tag.trim_start_matches('v'), &target.magic_hex()])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start the pinned CPython oracle");
            child
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            let expected = child.wait_with_output().unwrap();
            assert!(
                expected.status.success(),
                "{version:?}: {source}: {}",
                String::from_utf8_lossy(&expected.stderr)
            );
            let actual = compile_with_options(source, "paramspec.py", &options)
                .unwrap_or_else(|error| panic!("{version:?}: {source}: {error}"))
                .marshal();
            assert_eq!(actual, expected.stdout, "{version:?}: {source}");
        }
    }
}

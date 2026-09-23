mod async_generator_returns;
mod common;
mod except_star_transfers;

use std::fmt::Write as _;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

fn serc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_serc"))
}

#[test]
fn help_describes_compile_command() {
    let output = serc().arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Compile Python source to CPython bytecode."));
    assert!(stdout.contains("compile"));
}

#[test]
fn compile_writes_a_timestamp_pyc() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    let source = "answer = 42\n";
    fs_err::write(&source_path, source).unwrap();
    let source_mtime = 1_700_000_000;
    std::fs::File::options()
        .write(true)
        .open(&source_path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(UNIX_EPOCH + Duration::from_secs(u64::from(source_mtime))),
        )
        .unwrap();

    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let pyc_path = directory.path().join("__pycache__/module.cpython-314.pyc");
    let expected =
        serc::compile_to_pyc(source, &source_path.to_string_lossy(), source_mtime).unwrap();

    assert_eq!(fs_err::read(pyc_path).unwrap(), expected);
}

#[cfg(unix)]
#[test]
fn compile_matches_cpython_for_rounded_timestamps() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    let pyc_path = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source_path, "answer = 42\n").unwrap();

    for (before_epoch, seconds, nanos) in [
        (false, 1_700_000_000, 999_999_999),
        (false, 1_700_000_000, 1),
        (true, 1_700_000_000, 999_999_999),
        (true, 1_700_000_000, 1),
        (true, 1, 500_000_000),
        (true, 0, 500_000_000),
        (false, u64::from(u32::MAX), 999_999_999),
    ] {
        let duration = Duration::new(seconds, nanos);
        let modified = if before_epoch {
            UNIX_EPOCH - duration
        } else {
            UNIX_EPOCH + duration
        };
        std::fs::File::options()
            .write(true)
            .open(&source_path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(
            fs_err::metadata(&source_path).unwrap().modified().unwrap(),
            modified
        );

        let output = serc()
            .args(["compile", "--quiet"])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{modified:?}: {output:?}");

        let native = common::python_3145_command()
            .args([
                "-B",
                "-c",
                r"
import importlib._bootstrap_external as external
import pathlib
import py_compile
import sys

source = pathlib.Path(sys.argv[1])
cache = pathlib.Path(sys.argv[2])
actual = cache.read_bytes()
stat = source.stat()
external._validate_timestamp_pyc(actual, int(stat.st_mtime), stat.st_size, 'module', {})
oracle = py_compile.compile(str(source), cfile=str(cache) + '.oracle', doraise=True,
                            optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)
sys.stdout.buffer.write(pathlib.Path(oracle).read_bytes())
",
            ])
            .arg(&source_path)
            .arg(&pyc_path)
            .output()
            .unwrap();
        assert!(native.status.success(), "{modified:?}: {native:?}");
        assert_eq!(
            fs_err::read(&pyc_path).unwrap(),
            native.stdout,
            "{modified:?}"
        );
    }
}

#[test]
fn compile_keeps_python_versions_in_separate_caches() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    fs_err::write(&source_path, "answer = 42\n").unwrap();

    for version in [
        serc::PythonVersion::Py312,
        serc::PythonVersion::Py313,
        serc::PythonVersion::Py314,
        serc::PythonVersion::Py315,
    ] {
        let output = serc()
            .args([
                "compile",
                "--quiet",
                "--python-version",
                &version.to_string(),
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty());
        let (_, minor, _) = version.target().version;
        let pyc_path = directory
            .path()
            .join(format!("__pycache__/module.cpython-3{minor}.pyc"));
        let pyc = fs_err::read(pyc_path).unwrap();
        assert_eq!(&pyc[..4], &version.target().magic_number);
    }

    let legacy_path = directory.path().join("__pycache__/module.cpython-314.pyc");
    let legacy = fs_err::read(&legacy_path).unwrap();
    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(fs_err::read(legacy_path).unwrap(), legacy);
}

#[test]
fn compile_rejects_unsupported_python_version() {
    let output = serc()
        .args(["compile", "--python-version", "3.11", "--check"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unsupported Python version"));
    assert!(stderr.contains("3.12, 3.13, 3.14, or 3.15"));
}

#[test]
fn compile_preserves_template_not_equal_expression_text() {
    let source = r#"a, b, c = 1, 2, 3
cases = (
    (t"{(a != b):}", "(a != b)"),
    (t"{(a != b) != c:}", "(a != b)"),
    (t"{'!=' != 'other':}", "'!='"),
    (t"{f'{a != b}' != c:}", "f'{a != b}'"),
    (t"{[a != b] != c:}", "[a != b]"),
    (t"{a != b:}", "a"),
    (t"{a == b != c:}", "a == b"),
    (t"{a != (b != c):}", "a"),
    (t"{(a != b)!r:}", "(a != b)"),
    (t"{(a != b)}", "(a != b)"),
)
for template, expected in cases:
    assert template.interpolations[0].expression == expected
"#;
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("templates.py");
    fs_err::write(&source_path, source).unwrap();

    for (version, mut oracle) in [
        (serc::PythonVersion::Py314, common::python_3145_command()),
        (serc::PythonVersion::Py315, common::python_315_command()),
    ] {
        let native = oracle
            .args([
                "-B",
                "-c",
                "import marshal, pathlib, sys; path = pathlib.Path(sys.argv[1]); code = compile(path.read_bytes(), str(path), 'exec', dont_inherit=True, optimize=0); encoded = marshal.dumps(code); exec(code); sys.stdout.buffer.write(encoded)",
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(native.status.success(), "{version}: {native:?}");

        let compiled = serc::compile_with_options(
            source,
            &source_path.to_string_lossy(),
            &serc::CompileOptions {
                python_version: version,
                ..serc::CompileOptions::default()
            },
        )
        .unwrap();
        assert_eq!(compiled.marshal(), native.stdout, "Python {version}");
    }
}

#[test]
fn private_local_aliases_match_native_bindings_and_marshal() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("private_local_aliases.py");
    fs_err::write(
        &source_path,
        include_str!("../../serc/tests/fixtures/private_local_aliases.py"),
    )
    .unwrap();
    for version in [
        serc::PythonVersion::Py312,
        serc::PythonVersion::Py313,
        serc::PythonVersion::Py314,
        serc::PythonVersion::Py315,
    ] {
        let oracle = || match version {
            serc::PythonVersion::Py312 => common::python_312_command(),
            serc::PythonVersion::Py313 => common::python_313_command(),
            serc::PythonVersion::Py314 => common::python_3145_command(),
            serc::PythonVersion::Py315 => common::python_315_command(),
        };
        let output = serc()
            .args([
                "compile",
                "--quiet",
                "--python-version",
                &version.to_string(),
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{version}: {output:?}");
        let (_, minor, _) = version.target().version;
        let cache_path = directory.path().join(format!(
            "__pycache__/private_local_aliases.cpython-3{minor}.pyc"
        ));
        let native_path = directory.path().join(format!("native-3{minor}.pyc"));
        let native = oracle()
            .args([
                "-B",
                "-c",
                "import py_compile, sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)",
            ])
            .arg(&source_path)
            .arg(&native_path)
            .output()
            .unwrap();
        assert!(native.status.success(), "{version}: {native:?}");
        assert_eq!(
            fs_err::read(&cache_path).unwrap(),
            fs_err::read(&native_path).unwrap(),
            "Python {version}"
        );
        for artifact in [&native_path, &cache_path] {
            let output = oracle().arg("-B").arg(artifact).output().unwrap();
            assert!(
                output.status.success(),
                "{version}: {artifact:?}: {output:?}"
            );
        }
    }
}

#[test]
fn class_bindings_preserve_type_parameter_wrapper_cells() {
    for version in [
        serc::PythonVersion::Py312,
        serc::PythonVersion::Py313,
        serc::PythonVersion::Py314,
        serc::PythonVersion::Py315,
    ] {
        let mut sources = vec![
            "T = 'global'\nclass C:\n global T\n locals()['T'] = 'class'\n def f[T](arg: T): return T\n",
            "T = lambda: 'global'\nclass C:\n global T\n locals()['T'] = lambda: 'class'\n def f[T](arg: T()): return T\n",
            "T = 'global'\nclass C:\n global T\n locals()['T'] = 'class'\n def f[T: T](arg: T): return T\n",
            "T = int\nclass C:\n global T\n type Alias[T] = T\n",
            "T = int\nclass C:\n global T\n def method[T: T](): pass\n",
            "T = int\nclass C:\n global T\n class Inner[T: T]: pass\n",
            "class C:\n T = int\n def method[T](arg: T): return T\n",
            "class C:\n T = int\n type Alias[T] = T\n",
            "class C:\n T = int\n def method[T: T](): pass\n",
            "class C:\n T = int\n class Inner[T: T]: pass\n",
        ];
        if version >= serc::PythonVersion::Py313 {
            sources.extend([
                "T = int\nclass C:\n global T\n type Alias[T] = (T, lambda: T)\n",
                "T = int\nclass C:\n global T\n type Alias[T = T] = T\n",
                "T = int\nclass C:\n global T\n def method[T = T](): pass\n",
                "T = int\nclass C:\n global T\n class Inner[T = T]: pass\n",
                "T = int\nclass C:\n global T\n type Alias[T] = (T, [T for _ in (0,)])\n",
                "T = int\nclass C:\n global T\n type Alias[T] = [T for T in (0,)]\n",
                "T = int\nclass C:\n global T\n def method[T: (T, [T for _ in (0,)])](): pass\n",
                "T = int\nclass C:\n global T\n class Inner[T: (T, [T for _ in (0,)])]: pass\n",
                "class C:\n T = int\n type Alias[T] = (T, lambda: T, [T for _ in (0,)])\n",
                "class C:\n T = int\n type Alias[T = T] = T\n",
                "class C:\n T = int\n def method[T = T](): pass\n",
                "class C:\n T = int\n class Inner[T = T]: pass\n",
            ]);
        }
        for source in sources {
            let mut oracle = match version {
                serc::PythonVersion::Py312 => common::python_312_command(),
                serc::PythonVersion::Py313 => common::python_313_command(),
                serc::PythonVersion::Py314 => common::python_3145_command(),
                serc::PythonVersion::Py315 => common::python_315_command(),
            };
            let native = oracle
                .args([
                    "-c",
                    "import marshal, sys; code = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))",
                    source,
                    "generic_signature.py",
                ])
                .output()
                .unwrap();
            assert!(native.status.success(), "{version}: {source}: {native:?}");
            let compiled = serc::compile_with_options(
                source,
                "generic_signature.py",
                &serc::CompileOptions {
                    python_version: version,
                    ..serc::CompileOptions::default()
                },
            )
            .unwrap();
            assert_eq!(
                compiled.marshal(),
                native.stdout,
                "Python {version}: {source}"
            );
        }
    }
}

#[test]
fn compile_preserves_empty_sequence_repetition_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("empty_sequence_repetition.py");
    fs_err::write(
        &source_path,
        include_str!("../../serc/tests/fixtures/empty_sequence_repetition.py"),
    )
    .unwrap();

    for (version, mut oracle) in [
        (serc::PythonVersion::Py312, common::python_312_command()),
        (serc::PythonVersion::Py313, common::python_313_command()),
        (serc::PythonVersion::Py314, common::python_3145_command()),
        (serc::PythonVersion::Py315, common::python_315_command()),
    ] {
        let (_, minor, _) = version.target().version;
        let pyc_path = directory.path().join(format!(
            "__pycache__/empty_sequence_repetition.cpython-3{minor}.pyc"
        ));
        let native = oracle
            .args([
                "-B",
                "-c",
                "import py_compile, sys; py_compile.compile(sys.argv[1], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)",
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(native.status.success(), "{version}: {native:?}");
        let expected = fs_err::read(&pyc_path).unwrap();

        let output = serc()
            .args([
                "compile",
                "--quiet",
                "--python-version",
                &version.to_string(),
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{version}: {output:?}");
        assert_eq!(
            fs_err::read(&pyc_path).unwrap(),
            expected,
            "Python {version}"
        );

        let mut runtime = match version {
            serc::PythonVersion::Py312 => common::python_312_command(),
            serc::PythonVersion::Py313 => common::python_313_command(),
            serc::PythonVersion::Py314 => common::python_3145_command(),
            serc::PythonVersion::Py315 => common::python_315_command(),
        };
        let output = runtime.arg(&pyc_path).output().unwrap();
        assert!(output.status.success(), "{version}: {output:?}");
    }
}

#[test]
fn generic_class_headers_respect_enclosing_class_bindings() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("generic_class_header.py");
    let prelude =
        "class Base:\n    def __init_subclass__(cls, *, value):\n        cls.value = value\n";
    for version in [
        serc::PythonVersion::Py312,
        serc::PythonVersion::Py313,
        serc::PythonVersion::Py314,
        serc::PythonVersion::Py315,
    ] {
        let mut sources = Vec::new();
        for keyword in [false, true] {
            for (before, after, expected) in [
                ("        x = tuple\n        del x\n", "", "list"),
                ("", "        x = tuple\n", "list"),
                (
                    "        global x\n        locals()['x'] = tuple\n",
                    "",
                    "list",
                ),
                ("        x = tuple\n", "", "tuple"),
                ("", "", "dict"),
            ] {
                let (header, result) = if keyword {
                    ("Base, value=x", "C.D.value")
                } else {
                    ("x", "C.D.__bases__[0]")
                };
                sources.push(format!(
                    "{prelude}x = list\ndef outer(x):\n    class C:\n{before}        def keep(self): return x\n        class D[T]({header}): pass\n{after}    return C\nC = outer(dict)\nassert {result} is {expected}\nassert C.keep(None) is dict\n"
                ));
            }
            let (header, result) = if keyword {
                ("Base, value=x", "C.D.value")
            } else {
                ("x", "C.D.__bases__[0]")
            };
            sources.push(format!(
                "{prelude}x = list\ndef outer(x):\n    class C:\n        x = tuple\n        del x\n        class D[T]({header}):\n            def keep(self): return x\n    return C\nC = outer(dict)\nassert {result} is list\nassert C.D.keep(None) is dict\n"
            ));
            // CPython 3.12 rejects lambdas and comprehensions in class-visible annotation scopes.
            if version >= serc::PythonVersion::Py313 {
                for before in [
                    "        x = tuple\n        del x\n",
                    "        global x\n        locals()['x'] = tuple\n",
                ] {
                    for (expression, expected) in [
                        ("(lambda: x)()", "dict"),
                        ("(x, lambda: x)[0]", "list"),
                        ("[x for _ in (0,)][0]", "dict"),
                        ("(x, [x for _ in (0,)])[0]", "list"),
                        ("[x for x in (x,)][0]", "list"),
                        ("next(x for _ in (0,))", "dict"),
                        ("(x, (x for _ in (0,)))[0]", "list"),
                        ("next(item for item in (x,))", "list"),
                    ] {
                        let (header, result) = if keyword {
                            (format!("Base, value={expression}"), "C.D.value")
                        } else {
                            (expression.to_string(), "C.D.__bases__[0]")
                        };
                        for seed in ["", "        def keep(self): return x\n"] {
                            sources.push(format!(
                                "{prelude}x = list\ndef outer(x):\n    class C:\n{before}{seed}        class D[T]({header}): pass\n    return C\nC = outer(dict)\nassert {result} is {expected}\n"
                            ));
                        }
                    }
                }
            }
        }
        for (declaration, captured_direct) in [
            ("    T = int\n", "int"),
            ("    T = int\n    del T\n", "C.D.__type_params__[0]"),
            ("    global T\n", "C.D.__type_params__[0]"),
        ] {
            sources.push(format!(
                "{prelude}T = str\nclass C:\n{declaration}    class D[T](Base, value=T): pass\nassert C.D.value is C.D.__type_params__[0]\n"
            ));
            if version >= serc::PythonVersion::Py313 {
                sources.push(format!(
                    "{prelude}T = str\nclass C:\n{declaration}    class D[T](Base, value=(T, lambda: T)): pass\nassert C.D.value[0] is {captured_direct}\nassert C.D.value[1]() is C.D.__type_params__[0]\n"
                ));
            }
        }
        let (_, minor, _) = version.target().version;
        let pyc_path = directory.path().join(format!(
            "__pycache__/generic_class_header.cpython-3{minor}.pyc"
        ));
        let native_path = directory.path().join("native.pyc");
        let oracle = || match version {
            serc::PythonVersion::Py312 => common::python_312_command(),
            serc::PythonVersion::Py313 => common::python_313_command(),
            serc::PythonVersion::Py314 => common::python_3145_command(),
            serc::PythonVersion::Py315 => common::python_315_command(),
        };
        for source in sources {
            fs_err::write(&source_path, &source).unwrap();
            let output = serc()
                .args([
                    "compile",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                ])
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(output.status.success(), "{version}: {source}: {output:?}");
            let native = oracle()
                .args([
                    "-B", "-c",
                    "import py_compile, sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)",
                ])
                .arg(&source_path)
                .arg(&native_path)
                .output()
                .unwrap();
            assert!(native.status.success(), "{version}: {source}: {native:?}");
            assert_eq!(
                fs_err::read(&pyc_path).unwrap(),
                fs_err::read(&native_path).unwrap(),
                "Python {version}: {source}"
            );
            for artifact in [&native_path, &pyc_path] {
                let output = oracle().arg("-B").arg(artifact).output().unwrap();
                assert!(
                    output.status.success(),
                    "{version}: {artifact:?}: {source}: {output:?}"
                );
            }
        }
    }
}

#[test]
fn private_class_captures_match_native_marshal_and_execution() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("private_class_captures.py");

    for version in [
        serc::PythonVersion::Py312,
        serc::PythonVersion::Py313,
        serc::PythonVersion::Py314,
        serc::PythonVersion::Py315,
    ] {
        let mut source =
            include_str!("../../serc/tests/fixtures/private_class_captures.py").to_string();
        if version >= serc::PythonVersion::Py313 {
            source.push_str(include_str!(
                "../../serc/tests/fixtures/private_class_header_lambdas.py"
            ));
        }
        fs_err::write(&source_path, source).unwrap();
        let oracle = || match version {
            serc::PythonVersion::Py312 => common::python_312_command(),
            serc::PythonVersion::Py313 => common::python_313_command(),
            serc::PythonVersion::Py314 => common::python_3145_command(),
            serc::PythonVersion::Py315 => common::python_315_command(),
        };
        let native = oracle()
            .args([
                "-B",
                "-c",
                "import py_compile, sys; py_compile.compile(sys.argv[1], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)",
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(native.status.success(), "{version}: {native:?}");

        let (_, minor, _) = version.target().version;
        let pyc_path = directory.path().join(format!(
            "__pycache__/private_class_captures.cpython-3{minor}.pyc"
        ));
        let expected = fs_err::read(&pyc_path).unwrap();
        let native = oracle().arg(&pyc_path).output().unwrap();
        assert!(native.status.success(), "{version}: {native:?}");
        assert!(native.stdout.is_empty(), "{version}: {native:?}");

        let output = serc()
            .args([
                "compile",
                "--quiet",
                "--python-version",
                &version.to_string(),
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{version}: {output:?}");
        assert_eq!(
            fs_err::read(&pyc_path).unwrap(),
            expected,
            "Python {version}"
        );

        let output = oracle().arg(&pyc_path).output().unwrap();
        assert!(output.status.success(), "{version}: {output:?}");
        assert!(output.stdout.is_empty(), "{version}: {output:?}");
    }
}

#[test]
fn compile_matches_cpython_for_version_specific_marshal_identity() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    // Equal ordinary and slice constants must keep their distinct ownership and string tags.
    let mut scalar_source = String::new();
    for (index, value) in [
        "-2147483649",
        "-2147483648",
        "2147483647",
        "2147483648",
        "-9223372036854775808",
        "18446744073709551615",
        "18446744073709551616",
        "-18446744073709551616",
        "-0.0",
        "2 + 3j",
        "''",
        "'x'",
        "'é'",
        "'Ā'",
        "'identifier'",
        "'not interned'",
        r"'\ud800'",
        "b''",
        "b'x'",
        r"b'\xff\x00'",
        &format!("{:?}", "x".repeat(255)),
        &format!("{:?}", "x".repeat(256)),
    ]
    .into_iter()
    .enumerate()
    {
        // Constant slices currently mishandle empty-string identity and surrogate bounds.
        // Keep both scalar values covered through ordinary constants and tuples.
        if matches!(value, "''" | r"'\ud800'") {
            writeln!(
                scalar_source,
                "def scalar_{index}(a):\n    return {value}, ({value}, {value})"
            )
            .unwrap();
        } else {
            writeln!(scalar_source, "def scalar_{index}(a):\n    return {value}, a[{value}:{value}:{value}], a[{value}:{value}:{value}]").unwrap();
        }
    }
    let mut slice_source = String::new();
    for (sequence_index, sequence) in ["'aébc'", "b'abcd'", "(1, 2, 3, 4)", r"'a\ud800bc'"]
        .into_iter()
        .enumerate()
    {
        for (slice_index, slice) in [
            ":",
            "None:2",
            "1:None",
            "::None",
            "None:None:None",
            "None:3:2",
            "1:None:2",
            "1:3:None",
            "None:None:True",
            "False:None",
            "None:True",
            ":0:None",
            // Zero, negative, invalid, and dynamic components retain native runtime behavior.
            "None:None:0",
            "None:None:False",
            "None:None:-1",
            "-2:None",
            "None:-1",
            "None:2.0",
            "'start':None",
            "None:None:1.5",
            "value:None",
            "None:value",
            "None:None:value",
            "(None,)[0]:2",
            ":(None,)[0]",
            "None:None:(None,)[0]",
        ]
        .into_iter()
        .enumerate()
        {
            writeln!(
                slice_source,
                "def slice_{sequence_index}_{slice_index}(value):\n    return {sequence}[{slice}]"
            )
            .unwrap();
        }
    }
    for version in [
        serc::PythonVersion::Py312,
        serc::PythonVersion::Py313,
        serc::PythonVersion::Py314,
        serc::PythonVersion::Py315,
    ] {
        let (_, minor, _) = version.target().version;
        let pyc_path = directory
            .path()
            .join(format!("__pycache__/module.cpython-3{minor}.pyc"));
        let mut sources = vec![
            scalar_source.as_str(),
            slice_source.as_str(),
            include_str!("../../serc/tests/fixtures/generator_eager_headers.py"),
            "from __future__ import annotations\ndef outer():\n    def inner(value: (lambda: (yield 1))): pass\n    return inner\n",
            include_str!("../../serc/tests/fixtures/complex_division_signed_zeros.py"),
            include_str!("../../serc/tests/fixtures/nonfinite_complex_division.py"),
            "def answer():\n    return 42\n",
            "café = 'café'\n",
            "# coding: utf--8-sig\nvalue = 'é'\n",
            "# coding: utf__8_sig\nvalue = 'é'\n",
            "# coding: -utf-8-sig\nvalue = 'é'\n",
            "s = 'café'\nt = ('café',)\ndef f():\n    return café\n",
            "def f(a):\n    return a[257:1024:1000], 1025, -6, -5\n",
            "café = 'café' in {'café', 'autre'}\n",
            // Pruned ordinary strings still determine class qualified-name cache identity.
            "if False:\n    value = 'outer.<locals>.C'\ndef outer():\n    class C: pass\n    return C\n",
            "def discarded():\n    return None\n    value = 'outer.<locals>.C'\ndef outer():\n    class C: pass\n    return C\n",
            // Preserve annotation comprehension cells and target-specific qualified names.
            "def function(\n    sequence: [lambda: item for item in (1,)],\n    unique: {lambda: item for item in (2,)},\n) -> {key: lambda: key for key in (3,)}:\n    pass\n\ndef outer(values):\n    def inner(argument: [lambda: item for item in values]):\n        pass\n    return inner\n",
            "sequence: [lambda: item for item in (1,)]\nif True:\n    mapping: {key: lambda: key for key in (2,)}\nnested: [[lambda: (outer, inner) for inner in (3,)] for outer in (4,)]\n",
            "from __future__ import annotations\nsequence: [lambda: item for item in (1,)]\ndef function(argument: [lambda: item for item in (2,)]):\n    pass\n",
            "def f[T](x: [lambda: item for item in (1,)]): pass\n",
            "def outer(values):\n def f[T](x: [lambda: item for item in values]): pass\n return f\n",
            // Eager annotations keep ordinary comprehensions in the defining scope.
            "class C:\n def f(a: [i for i in (1,)]): pass\n",
            // Deferred module and class annotations inherit the active future flags.
            "from __future__ import barry_as_FLUFL\nvalue: int\n",
            "from __future__ import barry_as_FLUFL\nclass C:\n value: int\n def method(self, arg: int): pass\n",
            "from __future__ import barry_as_FLUFL\ndef outer():\n class C:\n  value: int\n return C\n",
            // Preserve legacy boolean inversion during recursive literal-aware folding.
            "value = ((~True,),)",
            r#"value = ((~False,), "\ud800")"#,
            // Recursive source-aware folding keeps target-specific tuple truth steps.
            r"value = not ()",
            r#"value = not ("\ud800",)"#,
            r#"value = ((not ("\ud800",)), "\udfff")"#,
            r"value = (not (),) * 2",
            r#"value = (not (), "\ud800")[0]"#,
            r"value = (17, 23)[not ()]",
            r#"value = (not ("\ud800",)) + 1"#,
            r#"value = (not (),) + ("\ud800",)"#,
            r#"value = not (runtime(), "\ud800")"#,
            // Deferred class-visible annotations preserve explicit global lookup.
            "x = 'global'\nclass C:\n global x\n locals()['x'] = 'class'\n def f(arg: x) -> x: pass\n",
            "x = 'global'\nclass C:\n x = 'class'\n def f(arg: x) -> x: pass\n",
            "x = 'global'\ndef outer():\n x = 'outer'\n class C:\n  global x\n  locals()['x'] = 'class'\n  def f(arg: x): pass\n return C\n",
            "T = 'global'\nclass C:\n global T\n locals()['T'] = 'class'\n def f[T](arg: T): pass\n",
            "T = 'global'\nclass C:\n global T\n locals()['T'] = 'class'\n def f[T](arg: T): return T\n",
            "def x(): return 'global'\nclass C:\n global x\n locals()['x'] = lambda: 'class'\n def f(arg: x()): pass\n",
            "x = 'global'\nclass C:\n global x\n locals()['x'] = 'class'\n field: x\n",
            // Preserve numeric allocation origins through the same recursive path.
            r#"value = (((1e309 - 1e309), "\ud800"),) * 2"#,
            r"value = (-(1e309 - 1e309), +(1e309 - 1e309))",
            // Preserve surrogate code points through composite constant folding.
            "value = \"\\ud800\"[0]",
            "value = \"\\ud800x\"[-1]",
            "value = \"a\\ud800b\\udfff\"[1:4:2]",
            "value = \"a\\ud800b\"[:1]",
            "value = \"\\ud800\"[:0]",
            "value = (\"\\ud800\",)[0]",
            "value = (\"\\ud800\", \"x\")[:1]",
            "value = (\"\\ud800\",) + (\"x\",)",
            "value = (\"\\ud800\",) * 2",
            "value = 2 * ((\"\\ud800\",),)",
            "value = (f\"\\ud800\",)",
            "value = f\"\\ud800\" + \"x\"",
            "value = (f\"\\ud800\" f\"\\udfff\")[1]",
            "value = (rf\"\\ud800\" f\"\\udfff\")[-1]",
            "value = item[\"\\ud800\":]",
            "value = item[:\"\\ud800\":]",
            "value = item[::\"\\ud800\"]",
            "value = item[r\"\\ud800\" \"\\udfff\":]",
            "item[\"\\ud800\":] = value",
            "del item[\"\\ud800\":]",
            // Discarded metadata can intern an equal tuple while leaving scalar literals plain.
            "if False:\n    def discarded():\n        return café\nvalue = ('café',)\n",
            "if False:\n    def discarded(café):\n        pass\nscalar = 'café'\nvalue = ('café',)\n",
            "if False:\n    def discarded(value: [café for café in ()]): pass\nvalue = ('format', 'café')\n",
            // Copied negation creates a fresh object at each assembler admission.
            "def first(flag, value):\n    return -((1e999 - 1e999) if flag else value)\ndef second(flag, value):\n    return -((1e999 - 1e999) if flag else value)\n",
            "def first(flag, value):\n    return -((1e999j - 1e999j) if flag else value)\ndef second(flag, value):\n    return -((1e999j - 1e999j) if flag else value)\n",
            "def first(flag, value):\n    return -(-((1e999 - 1e999) if flag else value))\ndef second(flag, value):\n    return -(-((1e999 - 1e999) if flag else value))\n",
            // Preserve NaN creation identity through distinct folds and copied constants.
            "a = 1e999 - 1e999\nb = 1e999 - 1e999\n",
            "a = {1e999 - 1e999, 1e999 - 1e999}\n",
            "a = ((1e999 - 1e999,), (1e999 - 1e999,))\n",
            "a = ((1e999 - 1e999,) * 2,) * 2\n",
            "a = (+(1e999 - 1e999), -(1e999 - 1e999))\n",
            "a = (1e999 - 1e999,)[0]\nb = 1e999 - 1e999\n",
            "a = 1e999j - 1e999j\nb = 1e999j - 1e999j\n",
            "a = {0.0 * 1e999j, 0.0 * 1e999j}\n",
            // Legacy mixed arithmetic first coerces scalars to complex values.
            "a = {1e999j * 0.0, 1e999j * 0.0}\n",
            "a = (0.0 * -0j, -0j * 0.0, (-0j) + 0.0, 0.0 + (-0j), 0.0 - 0j, 0j - 0.0)\n",
            "a = ((0.0 * 1e999j,),)[0]\nb = (1e999j * 0.0,) * 2\n",
            "a = (0 * 1e999j, False * 1e999j, 1e999j * 0, 1e999j * False)\n",
            // Exercise the legacy fused-product order with finite cancellation and input NaNs.
            "a = (1.0000000074505806 + 1j) * (0.9999999925494194 + 1j)\n",
            "a = (1 + 1.0000000074505806j) * (1 + 0.9999999925494194j)\n",
            "a = ((1e999 - 1e999) * 1j, 1j * (1e999 - 1e999), -(1e999 - 1e999) * 1j, 1j * -(1e999 - 1e999))\n",
            "def first():\n    return (1e999 - 1e999) + 1j\ndef second():\n    return (1e999 - 1e999) + 1j\n",
        ];
        if version >= serc::PythonVersion::Py313 {
            // Python 3.12 rejects comprehensions in class-visible annotation scopes.
            sources.extend([
                "class C:\n type A[T = {i for i in (1, 2)}] = T\n",
                "class C:\n type A = [[lambda: j for j in (i,)] for i in (1, 2)]\n",
                "class C:\n global __class__\n type A = [__class__ for i in (1,)]\n",
                "class C:\n global __class__\n type A = [i for i in __class__]\n",
                "def outer(a):\n class C:\n  a = type\n  class D[T](*[a for i in (1,)]): pass\n return C\n",
            ]);
        }
        if version >= serc::PythonVersion::Py314 {
            sources.extend([
                // Deferred signatures keep captures separate from direct class-global lookup.
                "class C:\n def f[T: [i for i in (1,)]](a: [T for i in (1,)]): pass\n",
                "def outer(x):\n class C:\n  x = None\n  def f(a: [x for i in (1,)]): pass\n return C\n",
                "def outer(x):\n class C:\n  x = None\n  def f[T](a: [x for i in (1,)]): pass\n return C\n",
                "def outer(x):\n class C:\n  global x\n  def f(a: (x, [x for i in (1,)])): pass\n return C\n",
                "class C:\n global T\n def f[T](a: (T, [T for i in (1,)])): pass\n",
                // Preserve empty-bytes identity across constant-cache admission orders.
                "value = b'x' * 0\n",
                "value = 0 * b'x'\n",
                "value = b'' * 3\n",
                "value = b'x'[:0]\n",
                "value = (b'x' * 0) * 2\n",
                "a = b''; value = b'x' * 0\n",
                "value = b'x' * 0; a = b''\n",
                "def f(): return b'x' * 0\nvalue = b''\n",
                "def f(): pass\nvalue = b'x' * 0\n",
                "value = b'x' * 0\ndef f(): pass\n",
                "value = (b'', b'x' * 0)\n",
                "value = (b'x' * 0, b'')\n",
                "value = b'x' * 0; other = data[b'':]\n",
                "other = data[:b'']; value = b'x' * 0\n",
                "if False:\n    def f(): return b'x' * 0\nvalue = b''\n",
                "def f():\n    return b'x' * 0\n    value = b''\n",
                // Discarded repetitions cannot certify early Boolean owner ordering.
                "'x' * False\nvalue = (1000 + 1, not 0)\n",
                "def f():\n    'x' * False\n    return (1000 + 1, not 0)\n",
                "'x' * 0\nvalue = (1000 + 1, not 0)\n",
                // Newly discarded folds retain annotation and class-footer locations.
                "'x' * False\nvalue: int\n",
                "False * 'x'\nvalue: int\n",
                "b'x' * False\nvalue: int\n",
                "(1,) * False\nvalue: int\n",
                "'x' * True\nvalue: int\n",
                "'x' * 0\nvalue: int\n",
                "class C:\n    ('x' * False)\n",
                "class C:\n    ('x' * 0)\n",
                "class C:\n    if ...:\n        ('x' * False)\n",
                "class C:\n    if ...:\n        ('x' * 0)\n",
                // Boolean repetition is enabled for the modern optimizer only.
                "value = 'text' * True\n",
                "value = False * 'text'\n",
                "value = '' * False\n",
                "value = b'bytes' * False\n",
                "value = True * b'bytes'\n",
                "value = False * b''\n",
                "value = (1,) * True\n",
                "value = False * (1,)\n",
                "value = True * ()\n",
                "value = \"\\ud800\" * False\n",
                "value = True * \"\\ud800\"\n",
                "value = (b'x' * False) or b'fallback'\n",
                "value = ('x' * True) and ('y' * False)\n",
                "value = (('x',) * True)[0]\n",
                "value = ((1,) * False,)[0]\n",
                "value = not (('x' * True,),)\n",
                // Repetition and tuple folding happen after the TO_BOOL sweep.
                "value = not ('x' * False)\n",
                "value = not (('x',) * True)\n",
                "value = not (b'x' * False)\n",
                "value = not not (b'x' * False)\n",
                "value = (not ('x' * False),)[0]\n",
                "value = not ()\n",
                "value = not ('x',)\n",
                "value = not (probe(),)\n",
                "def f():\n    return not (('x' * True,),)\n",
                "def f(value=b'x' * False, other='x' * True):\n    return value, other\n",
                "value = lambda arg=b'x' * False: arg\n",
                "if 'x' * False:\n    value = 1\nelse:\n    value = 2\n",
                "while () * False:\n    pass\n",
                "assert 'x' * True\n",
                "match ('x',) * True:\n    case ('x',):\n        value = 1\n",
                "match (b'x' * False,):\n    case (value,):\n        pass\n",
                "if False:\n    value = b'x' * False\nother = b''\n",
                "if False:\n    def f():\n        return b'x' * False\nother = b''\n",
                "value = False and (b'x' * False)\nother = b''\n",
                "match value:\n    case _ if ((\n        'x'\n    ) * True):\n        result = ((\n            'y'\n        ) * True)\n",
            ]);
        }
        for source in sources {
            fs_err::write(&source_path, source).unwrap();
            let mut oracle = match version {
                serc::PythonVersion::Py312 => common::python_312_command(),
                serc::PythonVersion::Py313 => common::python_313_command(),
                serc::PythonVersion::Py314 => common::python_3145_command(),
                serc::PythonVersion::Py315 => common::python_315_command(),
            };
            let output = oracle
                .args([
                    "-m",
                    "compileall",
                    "-q",
                    "-f",
                    "-o",
                    "0",
                    "--invalidation-mode",
                    "timestamp",
                ])
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            let expected = fs_err::read(&pyc_path).unwrap();

            let output = serc()
                .args([
                    "compile",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                ])
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                fs_err::read(&pyc_path).unwrap(),
                expected,
                "Python {version}: {source}"
            );
        }
    }
}

#[test]
fn check_compiles_a_directory_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let nested = directory.path().join("package");
    fs_err::create_dir_all(&nested).unwrap();
    fs_err::write(directory.path().join("module.py"), "value = 1\n").unwrap();
    fs_err::write(nested.join("__init__.py"), "from module import value\n").unwrap();

    for jobs in ["1", "4"] {
        let output = serc()
            .args(["compile", "--check", "--quiet", "--jobs", jobs])
            .arg(directory.path())
            .output()
            .unwrap();

        assert!(output.status.success(), "{output:?}");
        assert!(!directory.path().join("__pycache__").exists());
        assert!(!nested.join("__pycache__").exists());
    }
}

#[test]
fn parallel_compile_writes_identical_caches_including_colliding_names() {
    let directory = tempfile::tempdir().unwrap();
    let nested = directory.path().join("package");
    fs_err::create_dir_all(&nested).unwrap();
    for (path, source) in [
        (".py", "value = 'first cache owner'\n"),
        ("py.py", "value = 'last cache owner'\n"),
        ("module.py", "def value(x):\n    return x * 2\n"),
        ("package/__init__.py", "from .nested import value\n"),
        ("package/nested.py", "value = tuple(range(5))\n"),
        ("package/other.py", "value = {1, 2, 3}\n"),
    ] {
        fs_err::write(directory.path().join(path), source).unwrap();
    }
    let deeply_nested = format!("value = {}1{}\n", "call(".repeat(24), ")".repeat(24));
    fs_err::write(directory.path().join("deep.py"), deeply_nested).unwrap();
    let caches = [
        directory.path().join("__pycache__/py.cpython-314.pyc"),
        directory.path().join("__pycache__/module.cpython-314.pyc"),
        directory.path().join("__pycache__/deep.cpython-314.pyc"),
        nested.join("__pycache__/__init__.cpython-314.pyc"),
        nested.join("__pycache__/nested.cpython-314.pyc"),
        nested.join("__pycache__/other.cpython-314.pyc"),
    ];
    let serial = serc()
        .args(["compile", "--jobs", "1"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(serial.status.success(), "{serial:?}");
    assert_eq!(serial.stdout, b"Compiled 7 files\n");
    assert!(serial.stderr.is_empty());
    let expected = caches.each_ref().map(|path| fs_err::read(path).unwrap());

    for jobs in [Some("4"), None, Some("4")] {
        fs_err::remove_dir_all(directory.path().join("__pycache__")).unwrap();
        fs_err::remove_dir_all(nested.join("__pycache__")).unwrap();
        let mut command = serc();
        command.arg("compile").env("RAYON_NUM_THREADS", "2");
        if let Some(jobs) = jobs {
            command.args(["-j", jobs]);
        }
        let parallel = command.arg(directory.path()).output().unwrap();
        assert!(parallel.status.success(), "{parallel:?}");
        assert_eq!(parallel.stdout, serial.stdout);
        assert_eq!(parallel.stderr, serial.stderr);
        assert_eq!(
            caches.each_ref().map(|path| fs_err::read(path).unwrap()),
            expected
        );
    }
}

#[test]
fn parallel_workers_compile_deeply_nested_source_with_sufficient_stack() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("deep.py");
    let cache = directory.path().join("__pycache__");
    let output = cache.join("deep.cpython-314.pyc");
    let deeply_nested = format!("value = {}1{}\n", "call(".repeat(180), ")".repeat(180));
    fs_err::write(&source, deeply_nested).unwrap();
    fs_err::write(directory.path().join("other.py"), "value = 1\n").unwrap();

    let python = common::python_3145_command()
        .args(["-m", "py_compile"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(python.status.success(), "{python:?}");
    let expected = fs_err::read(&output).unwrap();

    for jobs in [Some("2"), None] {
        fs_err::remove_dir_all(&cache).unwrap();
        let mut command = serc();
        command
            .args(["compile", "--quiet"])
            .env("RAYON_NUM_THREADS", "2");
        if let Some(jobs) = jobs {
            command.args(["--jobs", jobs]);
        }
        let parallel = command.arg(directory.path()).output().unwrap();
        assert!(parallel.status.success(), "{parallel:?}");
        assert_eq!(fs_err::read(&output).unwrap(), expected);
    }
}

#[cfg(unix)]
#[test]
fn parallel_compile_orders_outputs_in_aliased_cache_directories() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("a");
    let last = directory.path().join("z");
    let alias = directory.path().join("b");
    let cache = directory.path().join("shared");
    for path in [&first, &last, &cache] {
        fs_err::create_dir(path).unwrap();
    }
    symlink(&first, &alias).unwrap();
    fs_err::write(first.join("module.py"), "value = 'first'\n").unwrap();
    fs_err::write(last.join("module.py"), "value = 'last'\n").unwrap();
    symlink(&cache, first.join("__pycache__")).unwrap();
    symlink(&cache, last.join("__pycache__")).unwrap();
    let output = cache.join("module.cpython-314.pyc");

    for sources in [
        [last.join("module.py"), first.join("module.py")],
        [alias.join("module.py"), first.join("module.py")],
    ] {
        let serial = serc()
            .args(["compile", "--jobs", "1"])
            .args(&sources)
            .output()
            .unwrap();
        assert!(serial.status.success(), "{serial:?}");
        let expected = fs_err::read(&output).unwrap();
        fs_err::remove_file(&output).unwrap();
        let parallel = serc()
            .args(["compile", "--jobs", "2"])
            .args(sources)
            .output()
            .unwrap();
        assert!(parallel.status.success(), "{parallel:?}");
        assert_eq!(parallel.stdout, serial.stdout);
        assert_eq!(parallel.stderr, serial.stderr);
        assert_eq!(fs_err::read(&output).unwrap(), expected);
    }

    fs_err::remove_file(first.join("__pycache__")).unwrap();
    let output = first.join("__pycache__/module.cpython-314.pyc");
    let sources = [alias.join("module.py"), first.join("module.py")];
    let serial = serc()
        .args(["compile", "--jobs", "1"])
        .args(&sources)
        .output()
        .unwrap();
    assert!(serial.status.success(), "{serial:?}");
    let expected = fs_err::read(&output).unwrap();
    fs_err::remove_dir_all(first.join("__pycache__")).unwrap();
    let parallel = serc()
        .args(["compile", "--jobs", "2"])
        .args(sources)
        .output()
        .unwrap();
    assert!(parallel.status.success(), "{parallel:?}");
    assert_eq!(fs_err::read(output).unwrap(), expected);
}

#[test]
fn parallel_compile_matches_serial_for_filesystem_case_and_unicode_aliases() {
    for (original, alias) in [
        ("A.py", "a.py"),
        ("K.py", "\u{212a}.py"),
        ("caf\u{e9}.py", "cafe\u{301}.py"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join(original);
        let second = directory.path().join(alias);
        let other = directory.path().join("other.py");
        fs_err::write(&first, "value = 1\n").unwrap();
        fs_err::write(&other, "value = 2\n").unwrap();
        if !second.is_file() {
            // Grouping itself is also checked on filesystems that distinguish these names.
            continue;
        }

        let inputs = [&second, &first, &other];
        let serial = serc()
            .args(["compile", "--jobs", "1"])
            .args(inputs)
            .output()
            .unwrap();
        assert!(serial.status.success(), "{serial:?}");
        let cache_directory = directory.path().join("__pycache__");
        let name = original.strip_suffix(".py").unwrap();
        let output = cache_directory.join(format!("{name}.cpython-314.pyc"));
        let expected = fs_err::read(&output).unwrap();
        for _ in 0..3 {
            fs_err::remove_dir_all(&cache_directory).unwrap();
            let parallel = serc()
                .args(["compile", "--jobs", "4"])
                .args(inputs)
                .output()
                .unwrap();
            assert!(parallel.status.success(), "{parallel:?}");
            assert_eq!(parallel.stdout, serial.stdout);
            assert_eq!(parallel.stderr, serial.stderr);
            assert_eq!(fs_err::read(&output).unwrap(), expected);
        }
    }
}

#[test]
fn parallel_compile_reports_errors_in_serial_order() {
    let directory = tempfile::tempdir().unwrap();
    for (name, source) in [
        ("z.py", "def broken(:\n"),
        ("m.py", "value = 42\n"),
        ("a.py", "if broken\n"),
    ] {
        fs_err::write(directory.path().join(name), source).unwrap();
    }
    let serial = serc()
        .args(["compile", "--check", "--jobs", "1"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert_eq!(serial.status.code(), Some(1));
    assert!(serial.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&serial.stderr);
    let lines: Vec<_> = stderr.lines().collect();
    assert_eq!(lines.len(), 3, "{stderr}");
    assert!(lines[0].contains("a.py"), "{stderr}");
    assert!(lines[1].contains("z.py"), "{stderr}");
    assert!(
        lines[2].contains("failed to compile 2 of 3 files"),
        "{stderr}"
    );

    for options in [
        ["--check", "-j", "4"].as_slice(),
        ["--check"].as_slice(),
        ["--jobs", "4"].as_slice(),
        [].as_slice(),
    ] {
        let parallel = serc()
            .arg("compile")
            .args(options)
            .env("RAYON_NUM_THREADS", "2")
            .arg(directory.path())
            .output()
            .unwrap();
        assert_eq!(parallel.status.code(), serial.status.code());
        assert_eq!(parallel.stdout, serial.stdout);
        assert_eq!(parallel.stderr, serial.stderr);
        assert_eq!(
            directory
                .path()
                .join("__pycache__/m.cpython-314.pyc")
                .exists(),
            !options.contains(&"--check")
        );
    }
}

#[test]
fn compile_rejects_zero_jobs() {
    let output = serc().args(["compile", "-j", "0"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid value '0'"), "{stderr}");
    assert!(stderr.contains("--jobs"), "{stderr}");
}

#[test]
fn compile_discovers_dot_py_basename() {
    let directory = tempfile::tempdir().unwrap();
    fs_err::write(directory.path().join(".py"), "answer = 42\n").unwrap();

    let output = serc()
        .args(["compile", "--quiet"])
        .arg(directory.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(
        directory
            .path()
            .join("__pycache__/py.cpython-314.pyc")
            .is_file()
    );
}

#[test]
fn compile_failure_has_a_nonzero_status() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("invalid.py");
    fs_err::write(&source_path, "if True print('no')\n").unwrap();

    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to compile"));
    assert!(stderr.contains("failed to parse Python source"));
}

#[test]
fn compile_rejects_null_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("null.py");
    fs_err::write(&source_path, b"value = 'before\0after'\n").unwrap();

    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("source code string cannot contain null bytes"));
    assert!(!directory.path().join("__pycache__").exists());
}

#[test]
fn compile_rejects_repeated_utf8_boms() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("repeated_bom.py");
    fs_err::write(&source_path, b"\xef\xbb\xbf\xef\xbb\xbfvalue = 1\n").unwrap();

    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("source contains multiple UTF-8 BOMs"));
    assert!(!directory.path().join("__pycache__").exists());
}

#[test]
fn compile_matches_cpython_cache_paths_and_pyc() {
    for name in [
        "module.py",
        "module",
        ".module",
        ".py",
        "some.module.py",
        ".module.py",
        "module.",
        "unicode_é.py",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join(name);
        fs_err::write(&source_path, "def answer():\n    return 42\n").unwrap();

        let cpython = common::python_3145_command()
            .args([
                "-X",
                "utf8",
                "-c",
                "import py_compile, sys; print(py_compile.compile(sys.argv[1], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP))",
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(cpython.status.success(), "{name}: {cpython:?}");
        let pyc_path = std::path::PathBuf::from(String::from_utf8(cpython.stdout).unwrap().trim());
        let expected = fs_err::read(&pyc_path).unwrap();
        fs_err::remove_file(&pyc_path).unwrap();

        let output = serc()
            .args(["compile", "--quiet"])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}: {output:?}");

        assert_eq!(fs_err::read(pyc_path).unwrap(), expected, "{name}");
    }
}

#[test]
fn compile_matches_cpython_for_utf8_encoding_cookies() {
    let directory = tempfile::tempdir().unwrap();

    for (name, source) in [
        ("utf8", b"# coding: utf-8\nvalue = '\xc3\xa9'\n".as_slice()),
        (
            "utf8_alias",
            b"# coding: utf8\nvalue = '\xc3\xa9'\n".as_slice(),
        ),
        (
            "utf8_bom",
            b"\xef\xbb\xbf# coding: utf-8\nvalue = '\xc3\xa9'\n".as_slice(),
        ),
    ] {
        let source_path = directory.path().join(format!("{name}.py"));
        let pyc_path = directory
            .path()
            .join(format!("__pycache__/{name}.cpython-314.pyc"));
        fs_err::write(&source_path, source).unwrap();

        let cpython = common::python_3145_command()
            .args([
                "-m",
                "compileall",
                "-q",
                "-f",
                "-o",
                "0",
                "--invalidation-mode",
                "timestamp",
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(cpython.status.success(), "{cpython:?}");
        let expected = fs_err::read(&pyc_path).unwrap();

        let output = serc()
            .args(["compile", "--quiet"])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");

        assert_eq!(fs_err::read(pyc_path).unwrap(), expected);
    }
}

#[test]
fn compile_accepts_ascii_cookies_and_rejects_non_ascii_source_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    fs_err::write(&source_path, b"# coding: ascii\nvalue = 'ascii'\n").unwrap();
    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    fs_err::write(&source_path, b"# coding: ascii\nvalue = '\xff'\n").unwrap();
    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to decode"));
}

#[test]
fn compile_rejects_unsupported_encoding_cookies() {
    let directory = tempfile::tempdir().unwrap();

    for encoding in ["cp932", "gbk", "big5"] {
        let source_path = directory.path().join(format!("{encoding}.py"));
        fs_err::write(
            &source_path,
            format!("# coding: {encoding}\nvalue = 'ascii'\n"),
        )
        .unwrap();

        let output = serc()
            .args(["compile", "--quiet"])
            .arg(&source_path)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!("unsupported source encoding `{encoding}`")),
            "{output:?}"
        );
    }
    assert!(!directory.path().join("__pycache__").exists());
}

#[cfg(unix)]
#[test]
fn compile_honors_umask_for_cache_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    let pyc_path = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source_path, "answer = 42\n").unwrap();
    fs_err::set_permissions(&source_path, std::fs::Permissions::from_mode(0o666)).unwrap();

    let output = Command::new("sh")
        .args([
            "-c",
            "umask 077; exec \"$1\" compile --quiet \"$2\"",
            "sh",
            env!("CARGO_BIN_EXE_serc"),
        ])
        .arg(&source_path)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let mode = fs_err::metadata(pyc_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[cfg(unix)]
#[test]
fn compile_replaces_a_cache_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    let cache_directory = directory.path().join("__pycache__");
    let pyc_path = cache_directory.join("module.cpython-314.pyc");
    let victim = directory.path().join("victim");
    fs_err::write(&source_path, "answer = 42\n").unwrap();
    fs_err::write(&victim, "keep me").unwrap();
    fs_err::create_dir(&cache_directory).unwrap();
    symlink(&victim, &pyc_path).unwrap();

    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(fs_err::read_to_string(victim).unwrap(), "keep me");
    assert!(fs_err::symlink_metadata(pyc_path).unwrap().is_file());
}

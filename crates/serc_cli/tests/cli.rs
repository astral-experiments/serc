mod common;

use std::fmt::Write as _;
use std::process::Command;
use std::time::UNIX_EPOCH;

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

    let output = serc()
        .args(["compile", "--quiet"])
        .arg(&source_path)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let pyc_path = directory.path().join("__pycache__/module.cpython-314.pyc");
    let metadata = fs_err::metadata(&source_path).unwrap();
    let seconds = metadata
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let source_mtime = u32::try_from(seconds).unwrap();
    let expected =
        serc::compile_to_pyc(source, &source_path.to_string_lossy(), source_mtime).unwrap();

    assert_eq!(fs_err::read(pyc_path).unwrap(), expected);
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
            "def answer():\n    return 42\n",
            "café = 'café'\n",
            "s = 'café'\nt = ('café',)\ndef f():\n    return café\n",
            "def f(a):\n    return a[257:1024:1000], 1025, -6, -5\n",
            "café = 'café' in {'café', 'autre'}\n",
            // Preserve annotation comprehension cells and target-specific qualified names.
            "def function(\n    sequence: [lambda: item for item in (1,)],\n    unique: {lambda: item for item in (2,)},\n) -> {key: lambda: key for key in (3,)}:\n    pass\n\ndef outer(values):\n    def inner(argument: [lambda: item for item in values]):\n        pass\n    return inner\n",
            "sequence: [lambda: item for item in (1,)]\nif True:\n    mapping: {key: lambda: key for key in (2,)}\nnested: [[lambda: (outer, inner) for inner in (3,)] for outer in (4,)]\n",
            "from __future__ import annotations\nsequence: [lambda: item for item in (1,)]\ndef function(argument: [lambda: item for item in (2,)]):\n    pass\n",
            "def f[T](x: [lambda: item for item in (1,)]): pass\n",
            "def outer(values):\n def f[T](x: [lambda: item for item in values]): pass\n return f\n",
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
        if version >= serc::PythonVersion::Py314 {
            sources.extend([
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

    let output = serc()
        .args(["compile", "--check", "--quiet"])
        .arg(directory.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(!directory.path().join("__pycache__").exists());
    assert!(!nested.join("__pycache__").exists());
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

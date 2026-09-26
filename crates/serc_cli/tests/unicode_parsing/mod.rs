use serc::{CompileError, CompileOptions, PythonVersion, compile_bytes_with_path_and_options};

use super::{common, serc};

#[test]
fn default_cli_preserves_native_unicode_parsing_identity() {
    let cases = [
        (
            "named_escape",
            "class UnitBase:\n    class EquivalentUnitsList:\n        pass\nmicro = '\\N{MICRO SIGN}'\n",
        ),
        (
            "normalized_identifier",
            "class Container:\n    class Entry:\n        pass\ncafé = 1\n",
        ),
        (
            "retained_members",
            "def first(value):\n    return value in {'normalize', 'unicode_marker_789'}, 1\ndef second(value):\n    'second docstring'\n    return value in {'normalize', 'unicode_marker_789'}, 2\nmicro = '\\N{MICRO SIGN}'\n",
        ),
        (
            "raw_escape_control",
            "class Container:\n    class Entry:\n        pass\ntext = r'\\N{MICRO SIGN}'\n",
        ),
    ];
    let directory = tempfile::tempdir().unwrap();
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (name, source) in cases {
            let source_path = directory.path().join(format!("{name}.py"));
            let native_path = directory.path().join("native.pyc");
            fs_err::write(&source_path, source).unwrap();
            let mut oracle = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = oracle
                .args([
                    "-B",
                    "-c",
                    "import py_compile, sys; assert 'unicodedata' not in sys.modules; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)",
                ])
                .arg(&source_path)
                .arg(&native_path)
                .output()
                .unwrap();
            assert!(native.status.success(), "{version} {name}: {native:?}");

            let compiled = serc()
                .args([
                    "compile",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                ])
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(compiled.status.success(), "{version} {name}: {compiled:?}");
            let (_, minor, _) = version.target().version;
            let actual_path = directory
                .path()
                .join(format!("__pycache__/{name}.cpython-3{minor}.pyc"));
            assert_eq!(
                fs_err::read(actual_path).unwrap(),
                fs_err::read(&native_path).unwrap(),
                "{version} {name}"
            );
        }
    }
}

#[test]
fn unicode_retry_hash_modes_preserve_raw_source_and_native_identity() {
    let source = b"\xef\xbb\xbfclass UnitBase:\r\n    class EquivalentUnitsList:\r\n        pass\r\nmicro = '\\N{MICRO SIGN}'\r\n";
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("unicode_hash.py");
    let native_path = directory.path().join("native.pyc");
    fs_err::write(&source_path, source).unwrap();

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
        assert!(
            matches!(
                compile_bytes_with_path_and_options(source, &source_path, &options),
                Err(CompileError::UnresolvedInternState { .. })
            ),
            "{version}: direct compilation did not require the Unicode retry"
        );
        let (_, minor, _) = version.target().version;
        let actual_path = directory
            .path()
            .join(format!("__pycache__/unicode_hash.cpython-3{minor}.pyc"));
        for (mode, native_mode) in [
            ("checked-hash", "CHECKED_HASH"),
            ("unchecked-hash", "UNCHECKED_HASH"),
        ] {
            let mut oracle = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = oracle
                .args([
                    "-B",
                    "-c",
                    "import py_compile, sys; assert 'unicodedata' not in sys.modules; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode[sys.argv[3]])",
                ])
                .arg(&source_path)
                .arg(&native_path)
                .arg(native_mode)
                .output()
                .unwrap();
            assert!(native.status.success(), "{version} {mode}: {native:?}");

            let compiled = serc()
                .args([
                    "compile",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                    "--invalidation-mode",
                    mode,
                ])
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(compiled.status.success(), "{version} {mode}: {compiled:?}");
            assert_eq!(
                fs_err::read(&actual_path).unwrap(),
                fs_err::read(&native_path).unwrap(),
                "{version} {mode}"
            );
        }
    }
}

#[test]
fn default_cli_keeps_unproved_unicode_and_warning_effects_unresolved() {
    for source in [
        "class UCD:\n    class CustomOwner:\n        pass\nmicro = '\\N{MICRO SIGN}'\n",
        "class Container:\n    class Entry:\n        pass\nmicro = '\\N{MICRO SIGN}'\nif 1 is 1:\n    pass\n",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unknown.py");
        fs_err::write(&path, source).unwrap();
        let compiled = serc()
            .args(["compile", "--quiet"])
            .arg(path)
            .output()
            .unwrap();
        assert!(!compiled.status.success(), "{compiled:?}");
        assert!(
            String::from_utf8_lossy(&compiled.stderr).contains("unresolved string intern state"),
            "{compiled:?}"
        );
        assert!(!directory.path().join("__pycache__").exists());
    }
}

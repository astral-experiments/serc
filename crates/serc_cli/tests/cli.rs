mod async_generator_returns;
mod common;
mod except_star_transfers;

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, UNIX_EPOCH};

fn serc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_serc"))
}

fn with_stdin(command: &mut Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn compile_file_lists_preserve_paths_and_deduplicate() {
    let directory = tempfile::tempdir().unwrap();
    fs_err::write(directory.path().join("first file.py"), "answer = 42\n").unwrap();
    fs_err::write(directory.path().join("second.py"), "pass\n").unwrap();
    fs_err::write(directory.path().join("ignored.py"), "invalid syntax\n").unwrap();

    let output = with_stdin(
        serc().current_dir(directory.path()).args([
            "compile",
            "--files-from",
            "-",
            "--null",
            "--jobs",
            "2",
            "--output-format",
            "json",
        ]),
        b"second.py\0first file.py\0second.py\0",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":2,\"skipped\":0}\n");

    let manifest = directory.path().join("inputs.txt");
    fs_err::write(&manifest, "second.py\nfirst file.py\r\nsecond.py\n").unwrap();
    let output = serc()
        .current_dir(directory.path())
        .args([
            "compile",
            "--files-from",
            "inputs.txt",
            "--output-format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":0,\"skipped\":2}\n");
}

#[test]
fn compile_empty_file_list_does_not_discover_current_directory() {
    let directory = tempfile::tempdir().unwrap();
    fs_err::write(directory.path().join("invalid.py"), "invalid syntax\n").unwrap();
    let output = with_stdin(
        serc().current_dir(directory.path()).args([
            "compile",
            "--files-from",
            "-",
            "--null",
            "--output-format",
            "json",
        ]),
        b"",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":0,\"skipped\":0}\n");
    assert!(!directory.path().join("__pycache__").exists());
}

#[test]
fn compile_file_list_rejects_directories_before_writing() {
    let directory = tempfile::tempdir().unwrap();
    fs_err::write(directory.path().join("module.py"), "pass\n").unwrap();
    fs_err::create_dir(directory.path().join("a")).unwrap();
    fs_err::create_dir(directory.path().join("z")).unwrap();
    let output = with_stdin(
        serc().current_dir(directory.path()).args([
            "compile",
            "--files-from",
            "-",
            "--null",
            "--jobs",
            "2",
        ]),
        b"z\0module.py\0a\0",
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("a is not a file"));
    assert!(!directory.path().join("__pycache__").exists());
}

#[cfg(unix)]
#[test]
fn compile_null_file_list_preserves_newline_names() {
    let directory = tempfile::tempdir().unwrap();
    fs_err::write(directory.path().join("new\nline.py"), "pass\n").unwrap();
    let output = with_stdin(
        serc().current_dir(directory.path()).args([
            "compile",
            "--files-from",
            "-",
            "--null",
            "--output-format",
            "json",
        ]),
        b"new\nline.py\0",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    assert!(
        directory
            .path()
            .join("__pycache__/new\nline.cpython-314.pyc")
            .is_file()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn compile_null_file_list_preserves_non_utf8_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let directory = tempfile::tempdir().unwrap();
    let name = OsStr::from_bytes(b"new\nline \xff.py");
    fs_err::write(directory.path().join(name), "pass\n").unwrap();
    let output = with_stdin(
        serc().current_dir(directory.path()).args([
            "compile",
            "--files-from",
            "-",
            "--null",
            "--output-format",
            "json",
        ]),
        b"new\nline \xff.py\0",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    assert!(
        directory
            .path()
            .join("__pycache__")
            .join(OsStr::from_bytes(b"new\nline \xff.cpython-314.pyc"))
            .is_file()
    );
}

#[cfg(windows)]
#[test]
fn compile_file_list_rejects_invalid_utf8() {
    let output = with_stdin(
        serc().args(["compile", "--files-from", "-", "--null"]),
        b"\xff.py\0",
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("file paths must be UTF-8"));
}

#[test]
fn compile_reuses_current_timestamp_caches() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("module.py");
    let cache = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source, "answer = 42\n").unwrap();
    let modified = fs_err::metadata(&source).unwrap().modified().unwrap();
    let output = serc()
        .args(["compile", "--output-format", "json"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    let original = fs_err::read(&cache).unwrap();
    let marker = UNIX_EPOCH + Duration::from_secs(1_000_000);
    fs_err::File::options()
        .write(true)
        .open(&cache)
        .unwrap()
        .set_modified(marker)
        .unwrap();

    fs_err::write(&source, "answer = 43\n").unwrap();
    fs_err::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let output = serc()
        .args(["compile", "--output-format", "json"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":0,\"skipped\":1}\n");
    assert_eq!(fs_err::read(&cache).unwrap(), original);
    assert_eq!(
        fs_err::metadata(&cache).unwrap().modified().unwrap(),
        marker
    );

    fs_err::write(&source, "answer = 430\n").unwrap();
    fs_err::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let output = serc()
        .args(["compile", "--output-format", "json"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");

    fs_err::write(&source, "answer = 431\n").unwrap();
    fs_err::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(modified + Duration::from_secs(2))
        .unwrap();
    let output = serc()
        .args(["compile", "--output-format", "json"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    assert_ne!(fs_err::read(&cache).unwrap(), original);
}

#[test]
fn compile_reuses_hash_caches_and_refreshes_changed_sources() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("module.py");
    let cache = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source, "answer = 42\n").unwrap();
    let mut command = serc();
    command
        .args(["compile", "--output-format", "json"])
        .arg(&source)
        .args(["--invalidation-mode", "checked-hash"]);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":0,\"skipped\":1}\n");
    assert_eq!(&fs_err::read(&cache).unwrap()[4..8], &[3, 0, 0, 0]);

    let mut command = serc();
    command
        .args(["compile", "--output-format", "json"])
        .arg(&source)
        .args(["--invalidation-mode", "unchecked-hash"]);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":0,\"skipped\":1}\n");
    assert_eq!(&fs_err::read(&cache).unwrap()[4..8], &[1, 0, 0, 0]);

    let modified = fs_err::metadata(&source).unwrap().modified().unwrap();
    fs_err::write(&source, "answer = 43\n").unwrap();
    fs_err::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    assert_eq!(
        &fs_err::read(&cache).unwrap()[8..16],
        serc::CPYTHON_TARGET.source_hash(b"answer = 43\n")
    );
}

#[test]
fn compile_refreshes_invalid_cache_headers() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("module.py");
    let cache = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source, "pass\n").unwrap();
    let mut command = serc();
    command
        .args(["compile", "--output-format", "json"])
        .arg(&source);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let expected = fs_err::read(&cache).unwrap();

    let mut wrong_magic = expected.clone();
    wrong_magic[0] = 0;
    fs_err::write(&cache, wrong_magic).unwrap();
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    assert_eq!(fs_err::read(&cache).unwrap(), expected);

    fs_err::write(&cache, &expected[..12]).unwrap();
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    assert_eq!(fs_err::read(&cache).unwrap(), expected);
}

#[test]
fn compile_force_rebuilds_current_cache() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("module.py");
    let cache = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source, "pass\n").unwrap();
    let output = serc().arg("compile").arg(&source).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"Compiled 1 file\n");
    let expected = fs_err::read(&cache).unwrap();
    let marker = UNIX_EPOCH + Duration::from_secs(1_000_000);
    fs_err::File::options()
        .write(true)
        .open(&cache)
        .unwrap()
        .set_modified(marker)
        .unwrap();

    let output = serc()
        .args(["compile", "--force", "--output-format", "json"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    assert_eq!(fs_err::read(&cache).unwrap(), expected);
    assert_ne!(
        fs_err::metadata(&cache).unwrap().modified().unwrap(),
        marker
    );
}

#[test]
fn check_compiles_current_caches_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("module.py");
    let cache = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source, "pass\n").unwrap();
    let output = serc().arg("compile").arg(&source).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let expected = fs_err::read(&cache).unwrap();
    let marker = UNIX_EPOCH + Duration::from_secs(1_000_000);
    fs_err::File::options()
        .write(true)
        .open(&cache)
        .unwrap()
        .set_modified(marker)
        .unwrap();

    let output = serc()
        .args(["compile", "--check", "--output-format", "json"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"compiled\":1,\"skipped\":0}\n");
    assert_eq!(fs_err::read(&cache).unwrap(), expected);
    assert_eq!(
        fs_err::metadata(&cache).unwrap().modified().unwrap(),
        marker
    );
}

/// Recompiles a source with the CLI while leaving its assertions to the caller.
fn compile_for_version(source: &Path, version: serc::PythonVersion) -> Output {
    serc()
        .args([
            "compile",
            "--force",
            "--quiet",
            "--python-version",
            &version.to_string(),
        ])
        .arg(source)
        .output()
        .unwrap()
}

/// Compares the complete timestamp cache, including its header, against CPython.
fn assert_compile_matches_cpython(
    source: &Path,
    native_path: &Path,
    version: serc::PythonVersion,
    oracle: &mut Command,
) -> PathBuf {
    let native = oracle
        .args([
            "-c",
            "import py_compile, sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)",
        ])
        .arg(source)
        .arg(native_path)
        .output()
        .unwrap();
    assert!(
        native.status.success(),
        "{version}: {}: {native:?}",
        source.display()
    );
    let compiled = compile_for_version(source, version);
    assert!(
        compiled.status.success(),
        "{version}: {}: {compiled:?}",
        source.display()
    );
    let (_, minor, _) = version.target().version;
    let cache_path = source.parent().unwrap().join(format!(
        "__pycache__/{}.cpython-3{minor}.pyc",
        source.file_stem().unwrap().to_str().unwrap()
    ));
    assert_eq!(
        fs_err::read(&cache_path).unwrap(),
        fs_err::read(native_path).unwrap(),
        "Python {version}: {}",
        source.display()
    );
    cache_path
}

/// Runs a compiled artifact with the caller's interpreter flags.
fn assert_executes(oracle: &mut Command, path: &Path, version: serc::PythonVersion) {
    let output = oracle.arg(path).output().unwrap();
    assert!(
        output.status.success(),
        "{version}: {}: {output:?}",
        path.display()
    );
}

#[test]
fn compile_matches_cpython_for_negated_ellipsis() {
    let cases = [
        "result = not ...\n",
        "def f():\n    return not ...\n",
        "result = not not ...\n",
        "result = not not not ...\n",
        "result = not not not not ...\n",
        "result = (not ..., ...)\n",
        "result = (not ..., not not ..., ...)\n",
        "result = True and not ...\n",
        "result = False or not not ...\n",
        "result = 1 and not ...\n",
        "result = 0 or not ...\n",
        "result = 257 and not ...\n",
        "result = None or not ...\n",
        "result = ... and not ...\n",
        "result = True and 257 and not ...\n",
        "def f():\n    return True and not ...\n",
        "def f():\n    return False or not not ...\n",
        "result = (\n    None or\n    not not ...\n)\n",
        "result = (\n    True and\n    not not ...\n)\n",
        "result = (\n    True and\n    not (\n        not ...\n    )\n)\n",
        "result = (\n    None or not (\n       not ...\n    )\n)\n",
        "result = ... and not not not ...\n",
        "result = ... and not not not not ...\n",
        "result = (\n    ... and\n    not not not ...\n)\n",
        "result = (\n    ... and\n    not not not not ...\n)\n",
        "result = ... and value\n",
        "result = ... or value\n",
        "result = value and ... and other\n",
        "not ...\n",
        "if not ...:\n    result = 1\nelse:\n    result = 2\n",
        "if not not ...:\n    result = 1\nelse:\n    result = 2\n",
        "def f():\n    while not ...:\n        return 1\n    return 2\n",
        "def f():\n    return (\n        not\n        ...\n    )\n",
        "result = (\n not (\n  not (\n   ...)))\n",
        "result = (\n not (\n  not (\n   not ...)))\n",
        "result = (\n not ...,\n not (\n  not ...),\n ...)\n",
        "result = (\n not (\n  not (\n   ...)),\n)\n",
        "result = (\n    not not ...,\n)\n",
        "result = (\n    not not ...,\n    not not ...,\n)\n",
        "result = (\n    (not not ...),\n)\n",
        "result = (\n    (not not ...,),\n)\n",
        "result = (\n    not ...,\n    not not ...,\n)\n",
        "result = ...\n",
        "if ...:\n    result = 1\nelse:\n    result = 2\n",
        "result = ... or 2\n",
        "result = not value\n",
        "result = not Ellipsis\n",
        "result = not ()\n",
    ];
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("negated_ellipsis.py");
    let native_path = directory.path().join("native.pyc");
    for version in common::PYTHON_VERSIONS {
        for source in cases {
            fs_err::write(&source_path, source).unwrap();
            let mut oracle = common::python_command(version);
            let native = oracle
                .args([
                    "-c",
                    "import py_compile, sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True, optimize=0)",
                ])
                .arg(&source_path)
                .arg(&native_path)
                .output()
                .unwrap();
            assert!(native.status.success(), "{version}: {source}: {native:?}");

            let compiled = compile_for_version(&source_path, version);
            assert!(
                compiled.status.success(),
                "{version}: {source}: {compiled:?}"
            );
            let (_, minor, _) = version.target().version;
            let pyc_path = directory
                .path()
                .join(format!("__pycache__/negated_ellipsis.cpython-3{minor}.pyc"));
            assert_eq!(
                fs_err::read(pyc_path).unwrap(),
                fs_err::read(&native_path).unwrap(),
                "{version}: {source}"
            );
        }
    }
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
fn compile_optimization_modes_match_cpython_and_use_distinct_caches() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("optimized.py");
    let native_path = directory.path().join("native.pyc");
    fs_err::write(
        &source_path,
        "\"module docstring\"\nassert __debug__\ndebug = __debug__\n\
         def function():\n    \"function docstring\"\n    assert condition\n    return __debug__\n\
         class Example:\n    \"class docstring\"\n    debug = __debug__\n",
    )
    .unwrap();

    let many_flags = format!("-{}", "O".repeat(256));
    for version in common::PYTHON_VERSIONS {
        for (level, arguments) in [
            ("0", vec![]),
            ("0", vec!["--optimize", "0"]),
            ("1", vec!["-O"]),
            ("1", vec!["--optimize", "1"]),
            ("2", vec!["-OO"]),
            ("2", vec!["--optimize", "2"]),
            ("3", vec!["-OOO"]),
            ("3", vec!["-O", "-OO"]),
            ("4", vec!["-OOOO"]),
            ("256", vec![many_flags.as_str()]),
        ] {
            let output = serc()
                .args([
                    "compile",
                    "--force",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                ])
                .args(&arguments)
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{version} {arguments:?}: {output:?}"
            );

            let mut oracle = common::python_command(version);
            oracle.arg("-B");
            if level.parse::<usize>().unwrap() > 2 {
                oracle.arg(format!("-{}", "O".repeat(level.parse().unwrap())));
            }
            let native = oracle
                .args([
                    "-c",
                    "import importlib.util, pathlib, py_compile, sys; source, native, level = sys.argv[1:]; level = int(level); py_compile.compile(source, cfile=native, doraise=True, optimize=level if level <= 2 else -1, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP); cache = importlib.util.cache_from_source(source, optimization='' if level == 0 else str(level)); assert pathlib.Path(cache).read_bytes() == pathlib.Path(native).read_bytes(), cache",
                ])
                .arg(&source_path)
                .arg(&native_path)
                .arg(level)
                .output()
                .unwrap();
            assert!(
                native.status.success(),
                "{version} {arguments:?}: {native:?}"
            );
        }
    }
    assert_eq!(
        fs_err::read_dir(directory.path().join("__pycache__"))
            .unwrap()
            .count(),
        24
    );
}

#[test]
fn optimized_check_mode_skips_cache_output() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    fs_err::write(&source_path, "\"docstring\"\nassert condition\n").unwrap();
    let output = serc()
        .args(["compile", "--check", "-OO"])
        .arg(source_path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!directory.path().join("__pycache__").exists());
}

#[test]
fn rejects_invalid_or_conflicting_optimization_options() {
    for arguments in [
        vec!["--optimize", "3"],
        vec!["-O", "--optimize", "2"],
        vec!["--optimize", "2", "-OOO"],
    ] {
        let output = serc().arg("compile").args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!output.stderr.is_empty());
    }
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

#[test]
fn compile_invalidation_modes_match_cpython() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    let oracle_path = directory.path().join("oracle.pyc");

    for version in common::PYTHON_VERSIONS {
        let (_, minor, _) = version.target().version;
        let pyc_path = directory
            .path()
            .join(format!("__pycache__/module.cpython-3{minor}.pyc"));

        for (source_name, source) in [
            ("plain", b"answer = 42\n".as_slice()),
            (
                "bom-crlf",
                b"\xef\xbb\xbf# coding: utf-8\r\nvalue = '\xc3\xa9'\r\n".as_slice(),
            ),
        ] {
            fs_err::write(&source_path, source).unwrap();
            for (mode, oracle_mode) in [
                ("timestamp", "TIMESTAMP"),
                ("checked-hash", "CHECKED_HASH"),
                ("unchecked-hash", "UNCHECKED_HASH"),
            ] {
                let mut oracle = common::python_command(version);
                let native = oracle
                    .args([
                        "-c",
                        "import py_compile, sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode[sys.argv[3]])",
                    ])
                    .arg(&source_path)
                    .arg(&oracle_path)
                    .arg(oracle_mode)
                    .output()
                    .unwrap();
                assert!(
                    native.status.success(),
                    "{version}, {source_name}, {mode}: {native:?}"
                );

                let output = serc()
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
                assert!(
                    output.status.success(),
                    "{version}, {source_name}, {mode}: {output:?}"
                );
                assert_eq!(
                    fs_err::read(&pyc_path).unwrap(),
                    fs_err::read(&oracle_path).unwrap(),
                    "Python {version}, {source_name}, {mode}"
                );
            }
        }
    }
}

#[test]
fn compile_rejects_invalid_invalidation_mode() {
    let output = serc()
        .args(["compile", "--invalidation-mode", "unknown", "--check"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalidation-mode"), "{stderr}");
    assert!(stderr.contains("unknown"), "{stderr}");
}

#[test]
fn hash_invalidation_modes_control_stale_cache_imports() {
    for (mode, expected) in [("checked-hash", "2\n"), ("unchecked-hash", "1\n")] {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("module.py");
        fs_err::write(&source_path, "value = 1\n").unwrap();

        let output = serc()
            .args(["compile", "--quiet", "--invalidation-mode", mode])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{mode}: {output:?}");

        fs_err::write(&source_path, "value = 2\n").unwrap();
        let imported = common::python_3145_command()
            .args([
                "-B",
                "-c",
                "import sys; sys.path.insert(0, sys.argv[1]); import module; print(module.value)",
            ])
            .arg(directory.path())
            .output()
            .unwrap();
        assert!(imported.status.success(), "{mode}: {imported:?}");
        assert_eq!(
            String::from_utf8_lossy(&imported.stdout).replace("\r\n", "\n"),
            expected,
            "{mode}"
        );
    }
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

    for version in common::PYTHON_VERSIONS {
        let output = compile_for_version(&source_path, version);
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
fn terminal_generator_assignments_match_cpython_315_marshal() {
    for callable in ["tuple", "all", "any", "list", "set"] {
        let source = format!("result = {callable}(x for x in values) if flag else ()\n");
        let native = common::python_315_command()
            .args([
                "-c",
                "import marshal, sys; code = compile(sys.argv[1], 'generator_assignment.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))",
                &source,
            ])
            .output()
            .unwrap();
        assert!(native.status.success(), "{callable}: {native:?}");
        let actual = serc::compile_with_options(
            &source,
            "generator_assignment.py",
            &serc::CompileOptions {
                python_version: serc::PythonVersion::Py315,
                ..serc::CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout, "{callable}");
    }
}

#[test]
fn unary_plus_patterns_match_cpython_315_marshal() {
    for pattern in [
        "+1",
        "+1.5",
        "+1j",
        "+1+2j",
        "-1",
        "{+1: +2}",
        "[+1, +2.5]",
        "+1 | +2",
    ] {
        let source = format!(
            "def f(subject):\n    match subject:\n        case {pattern}:\n            return True\n        case _:\n            return False\n"
        );
        let native = common::python_315_command()
            .args([
                "-c",
                "import marshal, sys; code = compile(sys.argv[1], 'patterns.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))",
                &source,
            ])
            .output()
            .unwrap();
        assert!(native.status.success(), "{pattern}: {native:?}");
        let compiled = serc::compile_with_options(
            &source,
            "patterns.py",
            &serc::CompileOptions {
                python_version: serc::PythonVersion::Py315,
                ..serc::CompileOptions::default()
            },
        )
        .unwrap();
        assert_eq!(compiled.marshal(), native.stdout, "{pattern}");
    }
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
fn private_nonlocal_aliases_match_native_bindings_and_pyc() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("private_nonlocal_aliases.py");
    fs_err::write(
        &source_path,
        include_str!("../../serc/tests/fixtures/private_nonlocal_aliases.py"),
    )
    .unwrap();
    for version in common::PYTHON_VERSIONS {
        let (_, minor, _) = version.target().version;
        let native_path = directory.path().join(format!("native-3{minor}.pyc"));
        let cache_path = assert_compile_matches_cpython(
            &source_path,
            &native_path,
            version,
            common::python_command(version).arg("-B"),
        );
        for artifact in [&native_path, &cache_path] {
            assert_executes(common::python_command(version).arg("-B"), artifact, version);
        }
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
    for version in common::PYTHON_VERSIONS {
        let (_, minor, _) = version.target().version;
        let native_path = directory.path().join(format!("native-3{minor}.pyc"));
        let cache_path = assert_compile_matches_cpython(
            &source_path,
            &native_path,
            version,
            common::python_command(version).arg("-B"),
        );
        for artifact in [&native_path, &cache_path] {
            assert_executes(common::python_command(version).arg("-B"), artifact, version);
        }
    }
}

#[test]
fn class_bindings_preserve_type_parameter_wrapper_cells() {
    for version in common::PYTHON_VERSIONS {
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
            let mut oracle = common::python_command(version);
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

    for version in common::PYTHON_VERSIONS {
        let mut oracle = common::python_command(version);
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

        let output = compile_for_version(&source_path, version);
        assert!(output.status.success(), "{version}: {output:?}");
        assert_eq!(
            fs_err::read(&pyc_path).unwrap(),
            expected,
            "Python {version}"
        );

        let mut runtime = common::python_command(version);
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
    for version in common::PYTHON_VERSIONS {
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
        let native_path = directory.path().join("native.pyc");
        for source in sources {
            fs_err::write(&source_path, &source).unwrap();
            let pyc_path = assert_compile_matches_cpython(
                &source_path,
                &native_path,
                version,
                common::python_command(version).arg("-B"),
            );
            for artifact in [&native_path, &pyc_path] {
                assert_executes(common::python_command(version).arg("-B"), artifact, version);
            }
        }
    }
}

#[test]
fn private_class_captures_match_native_marshal_and_execution() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("private_class_captures.py");

    for version in common::PYTHON_VERSIONS {
        let mut source =
            include_str!("../../serc/tests/fixtures/private_class_captures.py").to_string();
        if version >= serc::PythonVersion::Py313 {
            source.push_str(include_str!(
                "../../serc/tests/fixtures/private_class_header_lambdas.py"
            ));
        }
        fs_err::write(&source_path, source).unwrap();
        let oracle = || common::python_command(version);
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

        let output = compile_for_version(&source_path, version);
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
fn compile_matches_cpython_for_negative_empty_sequence_repetition() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory
        .path()
        .join("negative_empty_sequence_repetition.py");
    let mut sources = vec![
        include_str!("../../serc/tests/fixtures/negative_empty_sequence_repetition.py").to_string(),
        "prior = ('', 1)\nempty = '' * -1\nlater = (2, 3)\nreused = 'x' * -1\n".to_string(),
        "def f(flag):\n    if flag:\n        print('value')\n    return (1e1000 - 1e1000, '' * -1)\n".to_string(),
    ];
    // Synthetic constant containers and defaults have separate emission paths.
    for expression in [
        "x = ['' * -1, 2, 3]\n",
        "x = {'' * -1, 2, 3}\n",
        "x = ['' * -1, 2, 3, *values]\n",
        "for x in ['' * -1, 2, 3]: pass\n",
        "x = value in {'' * -1, 2, 3}\n",
        "def f(value='' * -1): pass\n",
        "f = lambda value='' * -1: value\n",
        "if '' * -1: print('unreachable')\nx = (2, 3)\n",
        "'' * -1\nx = (2, 3)\n",
        "x = '' * -1 or (2, 3)\n",
        "x: str = '' * -1\ny = (2, 3)\n",
    ] {
        sources.push(format!(
            "for repeat in ():\n    try:\n        repeat()\n    except Exception:\n        pass\n{expression}reused = 'text' * -1\n"
        ));
    }
    for source in sources {
        fs_err::write(&source_path, &source).unwrap();
        for version in common::PYTHON_VERSIONS {
            let mut oracle = common::python_command(version);
            let native = oracle
            .args([
                "-B",
                "-c",
                "import py_compile, sys; py_compile.compile(sys.argv[1], doraise=True, optimize=0, invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)",
            ])
            .arg(&source_path)
            .output()
            .unwrap();
            assert!(native.status.success(), "Python {version}: {native:?}");
            let (_, minor, _) = version.target().version;
            let pyc_path = directory.path().join(format!(
                "__pycache__/negative_empty_sequence_repetition.cpython-3{minor}.pyc"
            ));
            let expected = fs_err::read(&pyc_path).unwrap();
            let compiled = compile_for_version(&source_path, version);
            assert!(compiled.status.success(), "Python {version}: {compiled:?}");
            assert_eq!(
                fs_err::read(&pyc_path).unwrap(),
                expected,
                "Python {version}: {source}"
            );
        }
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
    for version in common::PYTHON_VERSIONS {
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
            let mut oracle = common::python_command(version);
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

            let output = compile_for_version(&source_path, version);
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
fn compile_matches_cpython_for_class_local_closure_shadowing() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    let native_path = directory.path().join("native.pyc");
    fs_err::write(
        &source_path,
        include_str!("../../serc/tests/fixtures/class_local_closure_shadow.py"),
    )
    .unwrap();

    for version in common::PYTHON_VERSIONS {
        let cache_path = assert_compile_matches_cpython(
            &source_path,
            &native_path,
            version,
            &mut common::python_command(version),
        );
        for artifact in [&native_path, &cache_path] {
            assert_executes(&mut common::python_command(version), artifact, version);
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
        for mode in ["timestamp", "checked-hash", "unchecked-hash"] {
            let output = serc()
                .args([
                    "compile",
                    "--check",
                    "--quiet",
                    "--jobs",
                    jobs,
                    "--invalidation-mode",
                    mode,
                ])
                .arg(directory.path())
                .output()
                .unwrap();

            assert!(output.status.success(), "{mode}: {output:?}");
            assert!(!directory.path().join("__pycache__").exists());
            assert!(!nested.join("__pycache__").exists());
        }
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
    for (optimization, arguments) in [
        ("0", vec!["--optimize", "0"]),
        ("1", vec!["--optimize", "1"]),
        ("2", vec!["--optimize", "2"]),
        ("3", vec!["-OOO"]),
    ] {
        let caches = [
            directory.path().join("__pycache__/py.cpython-314.pyc"),
            directory.path().join("__pycache__/module.cpython-314.pyc"),
            directory.path().join("__pycache__/deep.cpython-314.pyc"),
            nested.join("__pycache__/__init__.cpython-314.pyc"),
            nested.join("__pycache__/nested.cpython-314.pyc"),
            nested.join("__pycache__/other.cpython-314.pyc"),
        ]
        .map(|path| {
            if optimization == "0" {
                path
            } else {
                path.with_extension(format!("opt-{optimization}.pyc"))
            }
        });
        let serial = serc()
            .args(["compile", "--jobs", "1"])
            .args(&arguments)
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
            command
                .arg("compile")
                .args(&arguments)
                .env("RAYON_NUM_THREADS", "2");
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
            .args(["compile", "--force", "--jobs", "1"])
            .args(&sources)
            .output()
            .unwrap();
        assert!(serial.status.success(), "{serial:?}");
        let expected = fs_err::read(&output).unwrap();
        fs_err::remove_file(&output).unwrap();
        let parallel = serc()
            .args(["compile", "--force", "--jobs", "2"])
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
        .args(["compile", "--force", "--jobs", "1"])
        .args(&sources)
        .output()
        .unwrap();
    assert!(serial.status.success(), "{serial:?}");
    let expected = fs_err::read(&output).unwrap();
    fs_err::remove_dir_all(first.join("__pycache__")).unwrap();
    let parallel = serc()
        .args(["compile", "--force", "--jobs", "2"])
        .args(sources)
        .output()
        .unwrap();
    assert!(parallel.status.success(), "{parallel:?}");
    assert_eq!(fs_err::read(output).unwrap(), expected);
}

/// An earlier stale source can invalidate a later source's initially-current shared cache.
#[cfg(unix)]
#[test]
fn parallel_compile_rechecks_current_outputs_after_aliased_writes() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("a");
    let last = directory.path().join("z");
    let shared = directory.path().join("shared");
    fs_err::create_dir(&first).unwrap();
    fs_err::create_dir(&last).unwrap();
    fs_err::create_dir(&shared).unwrap();
    let first_source = first.join("module.py");
    let last_source = last.join("module.py");
    fs_err::write(&first_source, "value = 'first'\n").unwrap();
    fs_err::write(&last_source, "value = 'last source'\n").unwrap();
    symlink(&shared, first.join("__pycache__")).unwrap();
    symlink(&shared, last.join("__pycache__")).unwrap();
    let cache = shared.join("module.cpython-314.pyc");

    let seed = serc().arg("compile").arg(&last_source).output().unwrap();
    assert!(seed.status.success(), "{seed:?}");
    let expected = fs_err::read(&cache).unwrap();
    let manifest = format!("{}\0{}\0", last_source.display(), first_source.display());

    let serial = with_stdin(
        serc().args([
            "compile",
            "--files-from",
            "-",
            "--null",
            "--jobs",
            "1",
            "--output-format",
            "json",
        ]),
        manifest.as_bytes(),
    );
    assert!(serial.status.success(), "{serial:?}");
    assert_eq!(serial.stdout, b"{\"compiled\":2,\"skipped\":0}\n");
    assert_eq!(fs_err::read(&cache).unwrap(), expected);

    let parallel = with_stdin(
        serc().args([
            "compile",
            "--files-from",
            "-",
            "--null",
            "--jobs",
            "2",
            "--output-format",
            "json",
        ]),
        manifest.as_bytes(),
    );
    assert!(parallel.status.success(), "{parallel:?}");
    assert_eq!(parallel.stdout, b"{\"compiled\":2,\"skipped\":0}\n");
    assert_eq!(fs_err::read(&cache).unwrap(), expected);
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
            .args(["compile", "--force", "--jobs", "1"])
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
                .args(["compile", "--force", "--jobs", "4"])
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

#[cfg(unix)]
#[test]
fn compile_matches_cpython_for_non_utf8_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::Path;

    fn fixture_created(result: std::io::Result<()>, path: &Path) -> bool {
        // Darwin's EILSEQ is 92: APFS may refuse non-UTF-8 names, while Unix APIs can still
        // represent them. Other errors and filesystems that accept the fixtures must run the test.
        const DARWIN_EILSEQ: i32 = 92;
        match result {
            Ok(()) => true,
            Err(error)
                if cfg!(target_os = "macos") && error.raw_os_error() == Some(DARWIN_EILSEQ) =>
            {
                false
            }
            Err(error) => panic!("failed to create {path:?}: {error}"),
        }
    }

    fn oracle(version: serc::PythonVersion) -> Command {
        common::python_command(version)
    }

    let versions = common::PYTHON_VERSIONS;
    let directory = tempfile::tempdir().unwrap();
    let nested = directory
        .path()
        .join(OsString::from_vec(b"dir_\xf0\x9f\x90\xa7_\xfe".to_vec()));
    if !fixture_created(std::fs::create_dir(&nested), &nested) {
        return;
    }

    let stems = [
        b"caf\xc3\xa9_\xff_mid_\xe2\x82".as_slice(),
        b"replacement_\xef\xbf\xbd_\x80".as_slice(),
        b"sequence_\xed\xa0\x80_\xf0\x9f\x92".as_slice(),
        b"valid_\xc3\xa9".as_slice(),
    ];
    let mut paths = Vec::new();
    for stem in stems {
        let source_path = nested.join(OsString::from_vec([stem, b".py"].concat()));
        let source = common::python_3145_command()
            .args([
                "-X",
                "utf8",
                "-c",
                r"
import sys

filename = repr(sys.argv[1])
source = f'''literal = {filename}
def outer():
    def inner():
        return {filename}
    return inner
class Holder:
    def method(self):
        return {filename}
'''
sys.stdout.buffer.write(source.encode())
",
            ])
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(source.status.success(), "{source_path:?}: {source:?}");
        if !fixture_created(std::fs::write(&source_path, source.stdout), &source_path) {
            return;
        }
        std::fs::File::options()
            .write(true)
            .open(&source_path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            )
            .unwrap();

        for version in versions {
            let output = serc()
                .args(["compile", "--check", "--quiet", "--python-version"])
                .arg(version.to_string())
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "Python {version}, {source_path:?}: {output:?}"
            );
            assert!(output.stdout.is_empty(), "{source_path:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{source_path:?}: {output:?}");
        }
        paths.push((source_path, stem));
    }
    assert!(!nested.join("__pycache__").exists());

    for version in versions {
        let output = serc()
            .args(["compile", "--quiet", "--python-version"])
            .arg(version.to_string())
            .arg(directory.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "Python {version}: {output:?}");
        assert!(output.stdout.is_empty(), "Python {version}: {output:?}");
        assert!(output.stderr.is_empty(), "Python {version}: {output:?}");

        let (_, minor, _) = version.target().version;
        let suffix = format!(".cpython-3{minor}.pyc");
        for (index, (source_path, stem)) in paths.iter().enumerate() {
            let pyc_path = nested
                .join("__pycache__")
                .join(OsString::from_vec([*stem, suffix.as_bytes()].concat()));
            let oracle_path = directory
                .path()
                .join(format!("oracle-{version}-{index}.pyc"));
            let native = oracle(version)
                .args([
                    "-X",
                    "utf8",
                    "-c",
                    r"
import importlib.util
import marshal
import os
import pathlib
import py_compile
import sys
import types

source, cache, oracle = sys.argv[1:]
assert importlib.util.cache_from_source(source) == cache
py_compile.compile(source, cfile=oracle, doraise=True, optimize=0,
                   invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)

def check_filenames(path):
    code = marshal.loads(pathlib.Path(path).read_bytes()[16:])
    literal = next(value for value in code.co_consts if value == source)
    assert literal is not code.co_filename
    names = set()
    def check_child(child):
        names.add(child.co_name)
        assert os.fsencode(child.co_filename) == os.fsencode(source)
        assert child.co_filename is code.co_filename
        for value in child.co_consts:
            if isinstance(value, types.CodeType):
                check_child(value)
            elif value == source:
                assert value is literal
    check_child(code)
    assert names == {'<module>', 'outer', 'inner', 'Holder', 'method'}

check_filenames(oracle)
check_filenames(cache)
sys.stdout.buffer.write(pathlib.Path(oracle).read_bytes())
",
                ])
                .arg(source_path)
                .arg(&pyc_path)
                .arg(&oracle_path)
                .output()
                .unwrap();
            assert!(
                native.status.success(),
                "Python {version}, {source_path:?}: {native:?}"
            );
            assert_eq!(
                fs_err::read(pyc_path).unwrap(),
                native.stdout,
                "Python {version}, {source_path:?}"
            );
        }
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
            .args(["compile", "--force", "--quiet"])
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

    for encoding in ["cp932", "gbk", "koi8_u"] {
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

#[test]
fn compile_rejects_directory_destinations_even_when_forced() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("module.py");
    let cache = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source, "answer = 42\n").unwrap();
    fs_err::create_dir_all(&cache).unwrap();
    let marker = cache.join("keep.txt");
    fs_err::write(&marker, "keep me").unwrap();

    let output = serc()
        .args(["compile", "--force", "--quiet"])
        .arg(&source)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not a regular file"));
    assert_eq!(fs_err::read_to_string(&marker).unwrap(), "keep me");

    let output = serc()
        .args(["compile", "--check", "--quiet"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(fs_err::read_to_string(&marker).unwrap(), "keep me");
}

#[cfg(unix)]
#[test]
fn compile_rejects_cache_symlinks_with_current_headers() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("module.py");
    let cache = directory.path().join("__pycache__/module.cpython-314.pyc");
    let target = directory.path().join("target.pyc");
    fs_err::write(&source, "pass\n").unwrap();
    let mut command = serc();
    command
        .args(["compile", "--output-format", "json"])
        .arg(&source);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let expected = fs_err::read(&cache).unwrap();
    fs_err::rename(&cache, &target).unwrap();
    symlink(&target, &cache).unwrap();

    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not a regular file"));
    assert!(fs_err::symlink_metadata(&cache).unwrap().is_symlink());
    assert_eq!(fs_err::read(target).unwrap(), expected);
}

#[test]
fn compile_matches_cpython_for_class_comprehension_closure_slots() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    let native_path = directory.path().join("native.pyc");
    fs_err::write(
        &source_path,
        include_str!("../../serc/tests/fixtures/class_comprehension_closure_slots.py"),
    )
    .unwrap();

    for version in common::PYTHON_VERSIONS {
        let cache_path = assert_compile_matches_cpython(
            &source_path,
            &native_path,
            version,
            &mut common::python_command(version),
        );
        for artifact in [&native_path, &cache_path] {
            assert_executes(&mut common::python_command(version), artifact, version);
        }
    }
}

#[test]
fn compile_matches_cpython_for_legacy_class_comprehension_cells() {
    for before in [false, true] {
        let comprehension = "items = [lambda: x for x in ()]";
        let outside = "other = lambda: x";
        let (first, second) = if before {
            (outside, comprehension)
        } else {
            (comprehension, outside)
        };
        let source = format!(
            "def outer():\n    x = 'outside'\n    class C:\n        {first}\n        {second}\n    return C\n"
        );
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("module.py");
        let native_path = directory.path().join("native.pyc");
        fs_err::write(&source_path, source).unwrap();

        for version in common::PYTHON_VERSIONS {
            assert_compile_matches_cpython(
                &source_path,
                &native_path,
                version,
                &mut common::python_command(version),
            );
        }
    }
}

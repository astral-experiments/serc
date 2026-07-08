mod common;

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
fn compile_matches_cpython_pyc() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("module.py");
    let pyc_path = directory.path().join("__pycache__/module.cpython-314.pyc");
    fs_err::write(&source_path, "def answer():\n    return 42\n").unwrap();

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
fn compile_rejects_unsupported_encoding_cookies() {
    let directory = tempfile::tempdir().unwrap();

    for encoding in ["ascii", "cp932", "gbk", "big5"] {
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

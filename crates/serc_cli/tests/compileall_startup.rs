#![cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use serc::PythonVersion;

#[test]
fn normal_startup_preserves_constant_sharing_across_optimization_modes() {
    let cases = [
        "x = {'high', 'low', 'medium'}\ndef second(x):\n    return x in {'high', 'low', 'medium'}\n",
        "def first(x):\n    marker = 'ok'\n    return x in {'ok', 'succeeded'}\ndef second(x):\n    return x in {'ok', 'succeeded'} and 1\n",
        "def first(x):\n    marker = '__serc_unshared_first'\n    return x in {'__serc_unshared_first', '__serc_unshared_second'}\ndef second(x):\n    return x in {'__serc_unshared_first', '__serc_unshared_second'} and 1\n",
    ];
    let directory = tempfile::tempdir().unwrap();
    let native_cache = directory.path().join("native-cache");
    for (version, checked) in [
        (PythonVersion::Py312, common::python_312_command()),
        (PythonVersion::Py313, common::python_313_command()),
        (PythonVersion::Py314, common::python_3145_command()),
        (PythonVersion::Py315, common::python_315_command()),
    ] {
        let program = Path::new(checked.get_program());
        let executable = if program.is_absolute() || program.components().count() > 1 {
            fs_err::canonicalize(program).unwrap()
        } else {
            let path = std::env::var_os("PATH").expect("PATH must contain the checked interpreter");
            let found: PathBuf = std::env::split_paths(&path)
                .map(|directory| directory.join(program))
                .find(|candidate| candidate.is_file())
                .expect("checked interpreter must remain available");
            fs_err::canonicalize(found).unwrap()
        };
        for level in 0..=3 {
            let flags = format!("-{}", "O".repeat(level));
            let native_command = || {
                let mut command = Command::new(&executable);
                command
                    .current_dir(directory.path())
                    .env_clear()
                    .env("HOME", directory.path())
                    .env("PATH", "/usr/bin:/bin")
                    .env("LANG", "C.UTF-8")
                    .env("LC_ALL", "C.UTF-8")
                    .env("PYTHONPYCACHEPREFIX", &native_cache);
                if level != 0 {
                    command.arg(&flags);
                }
                command
            };
            // Each flag count has a distinct startup cache, including -OOO.
            let warmup = native_command()
                .args(["-m", "compileall", "--help"])
                .output()
                .unwrap();
            assert!(warmup.status.success(), "{version} {level}: {warmup:?}");
            for extension in ["py", "pyi"] {
                for (index, source) in cases.iter().enumerate() {
                    let filename = format!("sharing_{index}.{extension}");
                    fs_err::write(directory.path().join(&filename), source).unwrap();
                    let mut native = native_command();
                    if extension == "py" {
                        native.args([
                            "-m",
                            "compileall",
                            "-q",
                            "-f",
                            "--invalidation-mode",
                            "timestamp",
                        ]);
                    } else {
                        native.args(["-m", "py_compile"]);
                    }
                    let output = native.arg(&filename).output().unwrap();
                    assert!(output.status.success(), "{version} {level}: {output:?}");
                    let minor = version.target().version.1;
                    let cache_name = if level == 0 {
                        format!("sharing_{index}.cpython-3{minor}.pyc")
                    } else {
                        format!("sharing_{index}.cpython-3{minor}.opt-{level}.pyc")
                    };
                    let native_path = native_cache
                        .join(directory.path().strip_prefix("/").unwrap())
                        .join(&cache_name);
                    let expected = fs_err::read(&native_path).unwrap();
                    let mut compiler = Command::new(env!("CARGO_BIN_EXE_serc"));
                    compiler.current_dir(directory.path()).args([
                        "compile",
                        "--quiet",
                        "--python-version",
                        &version.to_string(),
                    ]);
                    if level != 0 {
                        compiler.arg(&flags);
                    }
                    let output = compiler.arg(&filename).output().unwrap();
                    assert!(output.status.success(), "{version} {level}: {output:?}");
                    assert_eq!(
                        fs_err::read(directory.path().join("__pycache__").join(cache_name))
                            .unwrap(),
                        expected,
                        "{version} {level} {filename}",
                    );
                }
            }
        }
    }
}

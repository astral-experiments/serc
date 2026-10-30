#[cfg(feature = "decompile")]
mod common;

use std::process::Command;

#[cfg(not(feature = "decompile"))]
#[test]
fn command_requires_feature() {
    let output = Command::new(env!("CARGO_BIN_EXE_serc"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("decompile")
    );
}

#[cfg(feature = "decompile")]
#[test]
fn complete_modules_only() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("original.py");
    let cache = temp.path().join("module.pyc");
    for version in common::PYTHON_VERSIONS {
        for optimize in 0..=2 {
            for supported in [true, false] {
                let text = if supported {
                    "def add(x, y):\n    return x + y\nanswer = add(20, 22)\n"
                } else {
                    "def unsupported(x):\n    try:\n        x()\n    except* Exception:\n        raise\n"
                };
                fs_err::write(&source, text).unwrap();
                let built = common::python_command(version).args(["-X", "no_debug_ranges", "-c", "import py_compile, sys; py_compile.compile(sys.argv[1], sys.argv[2], optimize=int(sys.argv[3]), doraise=True)"]).arg(&source).arg(&cache).arg(optimize.to_string()).output().unwrap();
                assert!(built.status.success(), "{built:?}");
                fs_err::remove_file(&source).unwrap();
                let recovered = Command::new(env!("CARGO_BIN_EXE_serc"))
                    .arg("decompile")
                    .arg(&cache)
                    .output()
                    .unwrap();
                if supported {
                    assert!(recovered.status.success(), "{version}: {recovered:?}");
                    assert_eq!(String::from_utf8(recovered.stdout).unwrap(), text);
                } else {
                    assert!(!recovered.status.success());
                    assert!(recovered.stdout.is_empty());
                    let message = String::from_utf8(recovered.stderr).unwrap();
                    assert!(
                        message.contains("unsupported")
                            && message.contains("byte")
                            && message.contains(&version.to_string()),
                        "{message}"
                    );
                }
            }
        }
    }
}

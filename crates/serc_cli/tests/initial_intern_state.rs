use std::process::Command;

use serc::{CompileOptions, OptimizationLevel, compile_bytes_with_path_and_options};

#[test]
fn cli_uses_the_library_initial_intern_state() {
    // Shared constants containing these strings depend on whether the caller
    // imported compileall or py_compile before compiling the source.
    let source = b"\
\"\"\"module documentation\"\"\"
x = {'high', 'low', 'medium'}
def first(x):
    marker = 'ok'
    assert __debug__
    return x in {'ok', 'succeeded'}
def second(x):
    return x in {'high', 'low', 'medium'} or x in {'ok', 'succeeded'}
";
    let directory = tempfile::tempdir().unwrap();
    for extension in ["py", "pyi"] {
        let filename = format!("sharing.{extension}");
        let path = std::path::Path::new(&filename);
        fs_err::write(directory.path().join(path), source).unwrap();
        for (flag, level, suffix) in [
            (None, OptimizationLevel::Zero, ""),
            (Some("-O"), OptimizationLevel::One, ".opt-1"),
            (Some("-OO"), OptimizationLevel::Two, ".opt-2"),
            (Some("-OOO"), OptimizationLevel::Two, ".opt-3"),
        ] {
            let expected = compile_bytes_with_path_and_options(
                source,
                path,
                &CompileOptions {
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .to_hash_pyc(source, true);

            let mut compiler = Command::new(env!("CARGO_BIN_EXE_serc"));
            compiler.current_dir(directory.path()).args([
                "compile",
                "--force",
                "--quiet",
                "--invalidation-mode",
                "checked-hash",
            ]);
            if let Some(flag) = flag {
                compiler.arg(flag);
            }
            let output = compiler.arg(path).output().unwrap();
            assert!(output.status.success(), "{filename} {flag:?}: {output:?}");
            let cache = directory
                .path()
                .join(format!("__pycache__/sharing.cpython-314{suffix}.pyc"));
            assert_eq!(
                fs_err::read(cache).unwrap(),
                expected,
                "{filename} {flag:?}",
            );
        }
    }
}

use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_with_options};

#[test]
fn copied_finalizers_preserve_python312_jump_width_boundaries() {
    let target = PythonVersion::Py312.target();
    let python = std::env::var_os("SERC_TEST_PYTHON_312").unwrap_or_else(|| "python3.12".into());
    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    let filename = "finally_loop_exit_jump_width.py";
    let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    for exit in ["break", "continue"] {
        for cleanup in ["finish()", "if flag:\n                finish()"] {
            for count in [123, 124, 125] {
                let mut source = format!(
                    "def f(items, flag):\n    for item in items:\n        try:\n            if flag:\n                {exit}\n        finally:\n            {cleanup}\n"
                );
                // Exceed the uniform seed bound, then put a backedge near 255.
                source.push_str(&"    if flag:\n        padding = 0\n".repeat(60));
                source.push_str(&"    padding = 0\n".repeat(150));
                source.push_str("    for item in items:\n");
                source.push_str(&"        padding = 0\n".repeat(count));
                source.push_str("        if flag:\n            work()\n");
                let mut child = Command::new(&python)
                    .args(["-I", "-S", "-c", oracle])
                    .args([
                        target.tag.trim_start_matches('v'),
                        &target.magic_hex(),
                        filename,
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(source.as_bytes())
                    .unwrap();
                let expected = child.wait_with_output().unwrap();
                assert!(
                    expected.status.success(),
                    "{}",
                    String::from_utf8_lossy(&expected.stderr)
                );
                assert_eq!(
                    compile_with_options(&source, filename, &options)
                        .unwrap()
                        .marshal(),
                    expected.stdout,
                    "{exit}: {cleanup}, {count} assignments"
                );
            }
        }
    }
}

use crate::common;

#[test]
fn matches_cpython_async_generator_folded_loop_cleanup() {
    for condition in ["False", "0", "not True"] {
        for prefix in ["", "consume(item)\n", "await wait(item)\n"] {
            let body =
                format!("{prefix}if {condition}:\n    yield item").replace('\n', "\n        ");
            let source =
                format!("async def f(values):\n    async for item in values:\n        {body}\n");
            common::assert_matches_cpython_marshal(&source, "async_generator_folded_cleanup.py");
        }
    }
}

#[test]
fn matches_cpython_live_cleanup_depth_after_dead_calls() {
    for (definition, suspension) in [
        ("async def", "await work(values)\n"),
        ("async def", "async for item in values:\n    pass\n"),
        ("def", "yield from values\n"),
    ] {
        for argument_count in [1, 2, 3, 5] {
            for branch in ["if False", "while False"] {
                let arguments = vec!["flag"; argument_count].join(", ");
                let body = format!("{suspension}{branch}:\n    consume({arguments})\n")
                    .replace('\n', "\n    ");
                let source = format!("{definition} f(values, flag):\n    {body}\n");
                common::assert_matches_cpython_marshal(&source, "live_cleanup_after_dead_call.py");
            }
        }
    }
}

#[test]
fn matches_cpython_folded_loop_cleanup_with_extended_arguments() {
    for (definition, body) in [
        ("def", "yield item"),
        ("async def", "yield item"),
        ("async def", "consume(item)"),
    ] {
        for loop_header in ["for item in values", "while flag"] {
            let prefix = "        consume(item)\n".repeat(60);
            let source = format!(
                "{definition} f(values, flag):\n    {loop_header}:\n{prefix}        if False:\n            {body}\n"
            );
            common::assert_matches_cpython_marshal(&source, "extended_folded_loop_cleanup.py");
        }
    }
}

#[test]
fn matches_cpython_315_removed_and_live_suspension_cleanup() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    use serc::{CompileOptions, PythonVersion, compile_with_options};

    let version = PythonVersion::Py315;
    let target = version.target();
    let python = std::env::var_os("SERC_TEST_PYTHON_315").unwrap_or_else(|| "python3.15".into());
    let script = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "suspension_cleanup_315.py";
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    for source in [
        "def f(values):\n    for value in values:\n        if False:\n            yield from values\n",
        "async def f(values):\n    for value in values:\n        if False:\n            await work(value)\n",
        "def f(values):\n    for value in values:\n        yield from values\n",
        "def f(values):\n    if False:\n        yield from values\n",
    ] {
        let mut oracle = Command::new(&python)
            .args(["-I", "-S", "-X", "utf8", "-c", script])
            .args([
                target.tag.strip_prefix('v').unwrap(),
                &target.magic_hex(),
                filename,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        oracle
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let expected = oracle.wait_with_output().unwrap();
        assert!(
            expected.status.success(),
            "CPython 3.15 oracle failed: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        assert_eq!(
            compile_with_options(source, filename, &options)
                .unwrap()
                .marshal(),
            expected.stdout,
            "marshal mismatch for {source}"
        );
    }
}

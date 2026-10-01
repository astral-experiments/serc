use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython(source: &str) {
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let filename = "coroutine_manager_borrow_boundaries.py";
        let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
        let mut child = Command::new(python)
            .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
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
            "{version:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for actual in [
            compile_with_options(source, filename, &options).unwrap(),
            compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
        ] {
            assert_eq!(actual.marshal(), expected.stdout, "{version:?}:\n{source}");
        }
    }
}

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

fn cleanup(binding: &str, suffix: &str) -> String {
    format!(
        "if flag:\n    with guard(){binding}:\n        await clean(value)\nelse:\n    await other(value)\n{suffix}"
    )
}

fn protected(body: &str, cleanup: &str) -> String {
    format!("try:\n{}finally:\n{}", indent(body), indent(cleanup))
}

const CONTINUATION: &str =
    "assert flag\nassert value\nassert not value.attr\nconsume(value, other)\nreturn value\n";

#[test]
fn coroutine_exceptional_manager_joins_interrupt_normal_borrow_fallthrough() {
    for body in ["work(value)", "await work(value)"] {
        for binding in ["", " as entered"] {
            for suffix in [
                "await finish(value)\n",
                "finish(value)\n",
                "value.attr = lambda: None\n",
                "assert flag\n",
                "assert await predicate(value)\n",
            ] {
                let suite = protected(body, &cleanup(binding, suffix)) + CONTINUATION;
                assert_matches_cpython(&format!(
                    "async def f(value, other, flag):\n{}",
                    indent(&suite)
                ));
            }
        }
    }
}

#[test]
fn coroutine_manager_exit_preserves_small_copy_and_retained_assertion_boundaries() {
    for suffix in [
        "await finish(value)\n",
        "finish(value)\n",
        "assert flag\n",
        "assert await predicate(value)\n",
    ] {
        for continuation in [
            "return\n",
            "return value\n",
            "raise Error()\n",
            "raise Error(value)\n",
            "assert await predicate(value)\nreturn value\n",
        ] {
            let suite = protected("work(value)", &cleanup("", suffix)) + continuation;
            assert_matches_cpython(&format!(
                "async def f(value, other, flag):\n{}",
                indent(&suite)
            ));
        }
    }
}

#[test]
fn coroutine_manager_exit_keeps_independent_incoming_branches_live() {
    for suffix in [
        "await finish(value)\n",
        "finish(value)\n",
        "assert flag\n",
        "assert await predicate(value)\n",
    ] {
        let guarded = protected("work(value)", &cleanup("", suffix));
        let suite = format!(
            "if skip:\n    prepare(value)\nelse:\n{}{CONTINUATION}",
            indent(&guarded)
        );
        assert_matches_cpython(&format!(
            "async def f(value, other, flag, skip):\n{}",
            indent(&suite)
        ));
    }
}

#[test]
fn coroutine_manager_alternatives_keep_linear_awaited_assertions() {
    for alternative in [
        "assert (await other(value)).attr == value",
        "assert (await other(value)) == value",
        "assert value == await other(value)",
        "assert check(await other(value)) == value",
    ] {
        for prefix in ["", "prepare(value)\n"] {
            for binding in ["", " as entered"] {
                let finalizer = cleanup(binding, "await finish(value)\n")
                    .replace("await other(value)", alternative);
                let finalizer = format!("{prefix}{finalizer}");
                let suite = protected("work(value)", &finalizer) + CONTINUATION;
                assert_matches_cpython(&format!(
                    "async def f(value, other, flag):\n{}",
                    indent(&suite),
                ));
            }
        }
    }
}

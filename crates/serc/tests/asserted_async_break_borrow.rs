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
        let filename = "asserted_async_break_borrow.py";
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

#[test]
fn asserted_async_break_preserves_the_empty_original_backedge() {
    for assertions in [
        "assert check(item)\n",
        "assert item.kind == other\n",
        "assert check(item)\nassert item.kind == other\n",
    ] {
        for manager in ["", "with manager:\n", "async with manager:\n"] {
            for captured in [false, true] {
                let loop_body = format!(
                    "async for item in stream:\n    if item:\n{}        break\nawait finish()\n",
                    indent(&indent(assertions)),
                );
                let protected = if manager.is_empty() {
                    loop_body
                } else {
                    format!("{manager}{}", indent(&loop_body))
                };
                let capture = if captured {
                    "    def capture():\n        return other, finish\n"
                } else {
                    ""
                };
                assert_matches_cpython(&format!(
                    "async def sample(stream, manager, other, check, finish):\n{capture}    try:\n{}    finally:\n        await stream.aclose()\n",
                    indent(&indent(&protected)),
                ));
            }
        }
    }
}

#[test]
fn asserted_async_break_preserves_nested_and_generator_contexts() {
    let loop_body = "async for item in stream:\n    if item:\n        assert check(item)\n        break\nawait finish()\n";
    for context in [
        "generator",
        "outer-for",
        "outer-while",
        "finally",
        "copied-finally",
    ] {
        let protected = match context {
            "outer-for" => format!("for outer in other:\n{}", indent(loop_body)),
            "outer-while" => format!("while other:\n{}    break\n", indent(loop_body)),
            "finally" => format!("try:\n    check(other)\nfinally:\n{}", indent(loop_body)),
            "copied-finally" => format!("try:\n    return\nfinally:\n{}", indent(loop_body)),
            _ => loop_body.to_owned(),
        };
        let prefix = if context == "generator" {
            "    yield other\n"
        } else {
            ""
        };
        assert_matches_cpython(&format!(
            "async def sample(stream, other, check, finish):\n{prefix}    try:\n{}    finally:\n        await stream.aclose()\n",
            indent(&indent(&protected)),
        ));
    }
}

#[test]
fn asserted_async_break_keeps_independent_entries_reachable() {
    for body in [
        "if other:\n    break\nif item:\n    assert check(item)\n    break\n",
        "if item:\n    assert check(item)\n    break\nawait finish()\n",
        "if item:\n    assert check(item)\n    break\nelse:\n    check(other)\n",
        "if item:\n    assert True\n    break\n",
        "if item:\n    assert False\n    break\n",
    ] {
        assert_matches_cpython(&format!(
            "async def sample(stream, other, check, finish):\n    try:\n        async for item in stream:\n{}        await finish()\n    finally:\n        await stream.aclose()\n",
            indent(&indent(&indent(body))),
        ));
    }
}

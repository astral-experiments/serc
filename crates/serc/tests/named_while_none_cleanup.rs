use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches(source: &str) {
    let filename = "named_while_cleanup.py";
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
        let mut child = Command::new(python)
            .args(["-B", "-I", "-S", "-c", oracle])
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
        for compiled in [
            compile_with_options(source, filename, &options).unwrap(),
            compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
        ] {
            assert!(
                compiled.marshal() == expected.stdout,
                "{version:?}:\n{source}"
            );
        }
    }
}

fn indent(source: &str) -> String {
    let mut indented = String::new();
    for line in source.lines() {
        indented.push_str("    ");
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}

#[test]
fn named_while_none_cleanup_preserves_original_break_copying() {
    for scope in ["def", "async def", "generator", "async generator"] {
        for context in ["plain", "loop", "manager", "async manager"] {
            if context == "async manager" && scope != "async def" {
                continue;
            }
            for condition in ["current is not None", "current", "current < limit"] {
                for guard in ["flag", "stop(current)"] {
                    let mut body = format!(
                        "try:\n    work(value)\nexcept Exception as error:\n    current = error\n    while {condition}:\n        if {guard}:\n            value = current\n            break\n        current = advance(current)\nwork(value)\n"
                    );
                    body = match context {
                        "loop" => format!("for item in items:\n{}", indent(&body)),
                        "manager" => format!("with manager:\n{}", indent(&body)),
                        "async manager" => format!("async with manager:\n{}", indent(&body)),
                        _ => body,
                    };
                    let prefix = if matches!(scope, "generator" | "async generator") {
                        body = format!("yield 0\n{body}");
                        if scope == "generator" {
                            "def"
                        } else {
                            "async def"
                        }
                    } else {
                        scope
                    };
                    body.push_str(if scope == "async generator" {
                        "return\n"
                    } else {
                        "return value\n"
                    });
                    assert_matches(&format!(
                        "{prefix} sample(value, flag, stop, advance, work, items, manager, limit):\n{}",
                        indent(&body)
                    ));
                }
            }
        }
    }
}

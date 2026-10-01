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
        let filename = "named_manager_borrow_boundaries.py";
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

fn protected(body: &str, recovery: &str, manager: &str, otherwise: &str) -> String {
    format!(
        "try:\n{}except Error as error:\n    if choose(other):\n{}    else:\n{}else:\n{}",
        indent(body),
        indent(&indent(recovery)),
        indent(&indent(manager)),
        indent(otherwise),
    )
}

fn function(suite: &str) -> String {
    format!("async def f(value, other, flag, skip):\n{}", indent(suite))
}

#[test]
fn named_final_arm_manager_cleanup_interrupts_normal_else_borrowing() {
    for body in ["await work(value)", "if flag:\n    await work(value)"] {
        for prefix in ["", "prepare(value)\n"] {
            for otherwise in ["value.state = other", "value.state = 1 if flag else 2"] {
                let manager = format!("{prefix}with manager():\n    recover(value)\n");
                let suite = protected(body, "await recover(value)", &manager, otherwise)
                    + "finish(value, other)\nreturn value\n";
                assert_matches_cpython(&function(&suite));
            }
        }
    }
}

#[test]
fn named_manager_cleanup_keeps_nested_guard_and_binding_identity() {
    for recovery in [
        "ready = await recover(value)\nif ready:\n    update(value)\nvalue.state = other",
        "ready = await recover(value)\nif ready:\n    update(value)\n    if other:\n        update(other)\nvalue.state = other",
    ] {
        for binding in ["", " as entered"] {
            let manager = format!("prepare(value)\nwith manager(){binding}:\n    recover(value)");
            let suite = protected(
                "if flag:\n    await work(value)",
                recovery,
                &manager,
                "value.state = 1 if flag else 2",
            ) + "finish(value, other)\nreturn value\n";
            assert_matches_cpython(&function(&suite));
            assert_matches_cpython(&function(
                &suite.replace("except Error as error:", "except (Error, Other) as error:"),
            ));
        }
    }
}

#[test]
fn named_manager_normal_exit_obeys_original_copy_width() {
    for continuation in [
        "return",
        "return value",
        "return other",
        "raise Error()",
        "raise Error(value)",
        "raise Error(value, other)",
        "return finish(value)",
        "return flag and value",
        "assert value\nreturn value",
        "assert flag and value\nreturn value",
        "if value:\n    finish(other)\nreturn value",
        "while flag:\n    finish(value)\nreturn value",
        "await finish(value)\nreturn value",
    ] {
        let suite = protected(
            "if flag:\n    await work(value)",
            "await recover(value)",
            "with manager():\n    recover(value)",
            "value.state = 1 if flag else 2",
        ) + continuation;
        assert_matches_cpython(&function(&suite));
    }
}

#[test]
fn named_manager_source_barrier_preserves_independent_incoming_edges() {
    for otherwise in ["value.state = other", "value.state = 1 if flag else 2"] {
        let guarded = protected(
            "if flag:\n    await work(value)",
            "await recover(value)",
            "with manager():\n    recover(value)",
            otherwise,
        );
        let suite = format!(
            "if skip:\n    prepare(value)\nelse:\n{}finish(value, other)\nreturn value\n",
            indent(&guarded),
        );
        assert_matches_cpython(&function(&suite));
    }
}

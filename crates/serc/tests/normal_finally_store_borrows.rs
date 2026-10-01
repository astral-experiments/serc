use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches(source: &str) {
    let filename = "normal_finally_store.py";
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
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

fn source(initial: &str, value: u8, keywords: bool, guarded: bool, branch: &str) -> String {
    let parameters = if initial == "argument" {
        ", result"
    } else {
        ""
    };
    let prologue = match initial {
        "assigned" => "result = None\n",
        "conditional" => "if condition:\n    result = None\n",
        "deleted" => "result = None\ndel result\n",
        "absent" | "argument" => "",
        _ => unreachable!(),
    };
    let discarded = if keywords {
        "operation(value=resource)"
    } else {
        "operation(resource)"
    };
    let mut recovery = String::from(
        "try:\n    resource.recover(error)\nexcept LookupError as other:\n    note(other)\n",
    );
    if guarded {
        recovery = format!("if condition:\n{}", indent(&recovery));
    }
    recovery = match branch {
        "plain" => format!(
            "if alternate:\n    note(error)\nelse:\n{}",
            indent(&recovery)
        ),
        "nested" => format!(
            "if alternate:\n    if condition:\n        note(error)\nelse:\n{}",
            indent(&recovery)
        ),
        "none" => recovery,
        _ => unreachable!(),
    };
    let body = format!(
        "{prologue}try:\n    operation()\n    if condition:\n        operation()\n    {discarded}\n    result = {value}\nexcept ValueError as error:\n{}finally:\n    try:\n        resource.close()\n    except ValueError as close_error:\n        note(close_error)\n        result = None\nreturn result\n",
        indent(&recovery)
    );
    format!(
        "def sample(resource, operation, note, condition, alternate{parameters}):\n{}",
        indent(&body)
    )
}

#[test]
fn populated_original_store_enters_guarded_normal_cleanup() {
    for initial in ["absent", "assigned", "argument", "conditional", "deleted"] {
        for value in [0, 1, 255] {
            for keywords in [false, true] {
                assert_matches(&source(initial, value, keywords, true, "none"));
            }
        }
    }
}

#[test]
fn unguarded_recovery_keeps_its_empty_original_cleanup_boundary() {
    for branch in ["none", "plain", "nested"] {
        for guarded in [false, true] {
            for keywords in [false, true] {
                assert_matches(&source("assigned", 0, keywords, guarded, branch));
            }
        }
    }
}

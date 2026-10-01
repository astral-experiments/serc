use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches(source: &str) {
    let filename = "completed_for_manager.py";
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

#[test]
fn completed_for_none_guard_retains_the_original_manager_owners() {
    for managers in ["manager0", "manager0, manager1"] {
        for loop_body in [
            "try:\n    visit(entry)\nexcept Error:\n    pass\n",
            "if token:\n    visit(entry)\n",
            "with manager1:\n    visit(entry)\n",
        ] {
            for condition in ["other", "probe(token)", "other == token"] {
                let finalbody = format!(
                    "for entry in entries:\n{}if flag is not None:\n    try:\n        if {condition}:\n            recover()\n    except Error:\n        pass\n",
                    indent(loop_body),
                );
                let body = format!("try:\n    work()\nfinally:\n{}", indent(&finalbody));
                let manager = format!("with {managers}:\n{}", indent(&body));
                assert_matches(&format!(
                    "def f(manager0, manager1, flag, other, token, entries):\n{}",
                    indent(&manager)
                ));
            }
        }
    }
}

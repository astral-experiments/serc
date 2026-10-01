use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches(source: &str) {
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
        let filename = "retained_finally_entry.py";
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
        for actual in [
            compile_with_options(source, filename, &options).unwrap(),
            compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
        ] {
            assert!(
                actual.marshal() == expected.stdout,
                "{version:?}:\n{source}"
            );
        }
    }
}

#[test]
fn retained_compound_exit_enters_only_its_populated_normal_finalizer() {
    for asynchronous in [false, true] {
        let prefix = if asynchronous { "async " } else { "" };
        let await_ = if asynchronous { "await " } else { "" };
        for early in [false, true] {
            let early = if early {
                "        if value.state == other.state:\n            return\n"
            } else {
                ""
            };
            for otherwise in [false, true] {
                let otherwise = if otherwise {
                    "            else:\n                value.state = True\n                finish(value)\n"
                } else {
                    ""
                };
                for finalizer in ["finish(value)", "value.queue.done()"] {
                    let source = format!(
                        "{prefix}def f(value, other, flag):\n    try:\n{early}        if flag:\n            try:\n                {await_}work(value)\n                value.state = True\n            except Inner:\n                value.state = False\n                raise\n{otherwise}            finally:\n                done(value)\n        else:\n            {await_}work(other)\n            value.state = False\n    except Error:\n        recover(value)\n    finally:\n        {finalizer}\n"
                    );
                    assert_matches(&source);
                }
            }
        }
    }
}

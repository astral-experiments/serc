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
        let filename = "guarded_recovery_borrows.py";
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
    let mut indented = String::new();
    for line in source.lines() {
        indented.push_str("    ");
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}

#[test]
fn populated_normal_source_survives_an_earlier_branch() {
    for definition in ["def", "async def"] {
        for action in ["obj.work()", "obj.work(timeout=2)", "value = obj.work()"] {
            for handler in ["except Error:", "except Error as error:"] {
                let body = format!(
                    "try:\n\
                     \x20   if obj.ready:\n\
                     \x20       pass\n\
                     \x20   {action}\n\
                     {handler}\n\
                     \x20   if obj.retry:\n\
                     \x20       try:\n\
                     \x20           obj.retry_work()\n\
                     \x20       except RetryError:\n\
                     \x20           obj.recover()\n\
                     for value in (obj.left, obj.right):\n\
                     \x20   obj.finish(value)\n"
                );
                assert_matches_cpython(&format!("{definition} probe(obj):\n{}", indent(&body)));
            }
        }
    }
}

#[test]
fn empty_normal_source_keeps_its_own_borrow_boundary() {
    for suffix in ["obj.finish()", "return obj.result"] {
        let source = format!(
            "def probe(obj):\n\
             \x20   try:\n\
             \x20       if obj.ready:\n\
             \x20           obj.work()\n\
             \x20   except Error:\n\
             \x20       if obj.retry:\n\
             \x20           try:\n\
             \x20               obj.retry_work()\n\
             \x20           except RetryError:\n\
             \x20               obj.recover()\n\
             \x20   {suffix}\n"
        );
        assert_matches_cpython(&source);
    }
}

#[test]
fn populated_while_jump_keeps_its_original_cleanup_route() {
    for definition in ["def", "async def"] {
        for condition in [
            "obj.active()",
            "obj.active() and not stopped() and not paused(obj.parent)",
        ] {
            for guard in ["finished", "not running and clock() - started >= limit"] {
                for work in ["pass", "wakeup.wait(0.025)"] {
                    for cleanup in ["obj.close()", "obj.close(missing_ok=True)"] {
                        let body = format!(
                            "try:\n\
                             \x20   while {condition}:\n\
                             \x20       if {guard}:\n\
                             \x20           break\n\
                             \x20       {work}\n\
                             except Error:\n\
                             \x20   pass\n\
                             finally:\n\
                             \x20   {cleanup}\n\
                             return 0\n"
                        );
                        assert_matches_cpython(&format!(
                            "{definition} probe(obj):\n{}",
                            indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

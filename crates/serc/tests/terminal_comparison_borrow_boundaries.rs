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
        let filename = "terminal_comparison_borrow_boundaries.py";
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
fn terminal_conjunction_returns_preserve_the_original_normal_exit_barrier() {
    for prefix in [
        "prepare(value)\n",
        "prepare(value)\nif other or check(value):\n    return value\nprepare(value)\n",
    ] {
        for condition in ["flag and a < b < c", "flag and a < middle(value) < c"] {
            for returned in ["value", "None"] {
                for exception in ["Error", "(Error, Other)"] {
                    for continuation in [
                        "consume(value)\nreturn value\n",
                        "if check(value):\n    consume(value)\nreturn value\n",
                    ] {
                        let body = format!("{prefix}if {condition}:\n    return {returned}\n");
                        let suite = format!(
                            "try:\n{}except {exception}:\n    pass\n{continuation}",
                            indent(&body),
                        );
                        assert_matches_cpython(&format!(
                            "def f(a, b, c, flag, other, value):\n{}",
                            indent(&suite),
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn neighboring_comparison_shapes_keep_small_return_copying() {
    for condition in [
        "flag and a < b < c",
        "flag and a < b < c < d",
        "flag and other and a < b < c",
        "a < b < c",
        "a < b < c and flag",
    ] {
        for binding in ["", " as error"] {
            let suite = format!(
                "try:\n    prepare(value)\n    if {condition}:\n        return value\nexcept Error{binding}:\n    pass\nreturn value\n"
            );
            assert_matches_cpython(&format!(
                "def f(a, b, c, d, flag, other, value):\n{}",
                indent(&suite),
            ));
        }
    }
    for binding in ["", " as error"] {
        assert_matches_cpython(&format!(
            "def f(a, b, c, flag, value):\n    try:\n        prepare(value)\n        if flag or a < b < c:\n            return value\n    except Error{binding}:\n        pass\n    consume(value)\n    return value\n"
        ));
    }
}

#[test]
fn independent_branches_still_enter_the_shared_continuation() {
    for bypass in ["pass", "prepare(value)"] {
        for returned in ["value", "None"] {
            for continuation in [
                "consume(value)\nreturn value\n",
                "if after:\n    consume(value)\nreturn value\n",
            ] {
                let guarded = format!(
                    "try:\n    prepare(value)\n    if flag and a < b < c:\n        return {returned}\nexcept Error:\n    pass\n"
                );
                let suite = format!(
                    "if skip:\n    {bypass}\nelse:\n{}{continuation}",
                    indent(&guarded),
                );
                assert_matches_cpython(&format!(
                    "def f(a, b, c, flag, skip, after, value):\n{}",
                    indent(&suite),
                ));
            }
        }
    }
}

#[test]
fn cleanup_and_suspending_scopes_keep_their_existing_small_exits() {
    let suite = "try:\n    prepare(value)\n    if flag and a < b < c:\n        return value\nexcept Error:\n    pass\nreturn value\n";
    for (prefix, body) in [
        ("async ", suite.to_owned()),
        ("", format!("yield value\n{suite}")),
        ("", format!("with manager:\n{}", indent(suite))),
        ("", format!("for item in items:\n{}", indent(suite))),
        ("", format!("try:\n    work()\nfinally:\n{}", indent(suite))),
        (
            "",
            format!("try:\n    work()\nexcept Other:\n{}", indent(suite)),
        ),
        (
            "",
            format!(
                "try:\n    work()\nexcept Other as error:\n{}",
                indent(suite)
            ),
        ),
    ] {
        assert_matches_cpython(&format!(
            "{prefix}def f(a, b, c, flag, value, manager, items):\n{}",
            indent(&body),
        ));
    }
}

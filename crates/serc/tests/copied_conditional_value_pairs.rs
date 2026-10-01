use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython(source: &str, version: PythonVersion) {
    let target = version.target();
    let (variable, fallback) = match version {
        PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12"),
        PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13"),
        PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
        PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
    };
    let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "copied_conditional_value_pairs.py";
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
        "CPython oracle failed: {}",
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
        assert_eq!(actual.marshal(), expected.stdout, "{version:?}: {source}");
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
fn copied_local_pairs_preserve_original_array_and_owner_membership() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for condition in ["flag", "value is not None"] {
            for fallback in ["{}", "fallback()"] {
                let value = format!("value if {condition} else {fallback}");
                for statement in [
                    format!("return ({value}) >= other\n"),
                    format!("return other, {value}, self\n"),
                    format!("self.result: dict = {value}\n"),
                    format!("self.result = other or ({value})\n"),
                ] {
                    for asynchronous in [false, true] {
                        for context in ["plain", "completed", "completed_finally", "protected"] {
                            let mut suite = String::new();
                            if asynchronous {
                                suite.push_str("await start()\n");
                            }
                            match context {
                                "completed" => {
                                    suite.push_str(
                                        "try:\n    prepare()\nexcept Error:\n    recover()\n",
                                    );
                                    suite.push_str(&statement);
                                }
                                "completed_finally" => {
                                    suite.push_str("try:\n    prepare()\nfinally:\n    finish()\n");
                                    suite.push_str(&statement);
                                }
                                "protected" => {
                                    suite.push_str("try:\n");
                                    suite.push_str(&indent(&statement));
                                    suite.push_str("except Error:\n    recover()\n");
                                }
                                _ => suite.push_str(&statement),
                            }
                            let prefix = if asynchronous { "async " } else { "" };
                            let source = format!(
                                "{prefix}def f(self, value, other, flag):\n{}",
                                indent(&suite)
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn copied_chained_conditional_arms_keep_their_original_jump_slots() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (prefix, suspension) in [
            ("", ""),
            ("async ", "    await start()\n"),
            ("", "    yield value\n"),
        ] {
            for (body, otherwise) in [
                ("a <= b < c", "a < b <= c"),
                ("a <= b < c", "b >= a or b < c"),
                ("a <= b", "a < b <= c"),
                ("a <= b < c", "a < b"),
                ("a <= b", "b < c"),
                ("a < b < c < value", "a < b <= c"),
            ] {
                for multiline in [false, true] {
                    let condition = if multiline {
                        format!("\n        {body}\n        if flag\n        else {otherwise}\n    ")
                    } else {
                        format!("{body} if flag else {otherwise}")
                    };
                    for preceding in ["", "value or "] {
                        let source = format!(
                            "{prefix}def f(a, b, c, flag, value):\n{suspension}    if {preceding}not ({condition}):\n        return False\n    return True\n"
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn chained_conditional_arm_policy_is_scoped_to_the_original_comparison() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (prefix, suspension) in [
            ("", ""),
            ("async ", "    await start()\n"),
            ("", "    yield value\n"),
        ] {
            for condition in [
                "(a <= b < c if flag else a < b <= c) if other else a < b",
                "a <= (b if other else value) < c if flag else a < b",
                "a <= b < c if flag else (a < b <= c if other else a < b)",
                "(a <= b < c if flag else a < b) or other",
                "other or (a <= b < c if flag else a < b)",
                "(a <= b < c if flag else a < b) and other",
                "other and (a <= b < c if flag else a < b)",
                "(a <= b < c) if (flag if other else value) else a < b",
            ] {
                for following in ["", "    if a <= b < c:\n        return True\n"] {
                    let source = format!(
                        "{prefix}def f(a, b, c, flag, value, other):\n{suspension}    if not ({condition}):\n        return build(a, b, c, flag, value, other)\n{following}    return None\n"
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn chained_conditional_arm_retention_preserves_noncopying_targets() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (body, otherwise) in [
            ("0 <= len(value) <= limit", "0 < len(value) <= limit"),
            (
                "a.first <= b.middle < c.last",
                "a.first < b.middle <= c.last",
            ),
            ("a is b is None", "a is b is not None"),
        ] {
            for multiline in [false, true] {
                let condition = if multiline {
                    format!("\n        {body}\n        if flag\n        else {otherwise}\n    ")
                } else {
                    format!("{body} if flag else {otherwise}")
                };
                for (prefix, suspension, action, tail) in [
                    ("", "", "return False", "return True"),
                    ("async ", "    yield value\n", "return", "return"),
                    ("", "", "consume(value)", "return True"),
                    ("", "", "return build(a, b, c, flag, value)", "return True"),
                ] {
                    let source = format!(
                        "{prefix}def f(a, b, c, flag, value):\n{suspension}    if not ({condition}):\n        {action}\n    {tail}\n"
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn chained_conditional_arms_preserve_comprehension_and_suspension_contexts() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (condition, result) in [
            ("a < b < c if flag else a < b", "False"),
            (
                "(a < b < c if flag else a < b <= c) if other else (a < b < c if value else a < b)",
                "build(a, b, c, flag, value, other)",
            ),
        ] {
            for iteration in ["for", "async for"] {
                let prefix = if iteration == "for" { "" } else { "async " };
                for (left, right) in [("[", "]"), ("(", ")")] {
                    for tested in [format!("({condition})"), format!("not ({condition})")] {
                        for expression in [
                            format!("item {iteration} item in items if {tested} or other"),
                            format!("value if {tested} else other {iteration} item in items"),
                        ] {
                            let source = format!(
                                "{prefix}def f(a, b, c, flag, value, other, items):\n    return {left}{expression}{right}\n"
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
            for (prefix, operand) in [("async ", "(await middle())"), ("", "(yield value)")] {
                let condition = condition.replace('b', operand);
                let source = format!(
                    "{prefix}def f(a, b, c, flag, value, other, items):\n    if not ({condition}):\n        return {result}\n    return True\n"
                );
                assert_matches_cpython(&source, version);
            }
            let source = format!(
                "def f(a, b, c, flag, value, other):\n    try:\n        work()\n    finally:\n        if not ({condition}):\n            return {result}\n    return True\n"
            );
            assert_matches_cpython(&source, version);
        }
    }
}

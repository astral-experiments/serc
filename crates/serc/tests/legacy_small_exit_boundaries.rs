use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython(source: &str) {
    assert_matches_cpython_version(source, PythonVersion::Py312);
}

fn assert_matches_cpython_version(source: &str, version: PythonVersion) {
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
    let filename = "legacy_small_exit_boundaries.py";
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

#[test]
fn inner_handlers_retire_before_exceptional_finalizers_with_ambient_cleanup() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix) in [("def", ""), ("async def", ""), ("def", "yield value\n")] {
            for headers in [
                &[][..],
                &["for item in items:"][..],
                &["while flag:"][..],
                &["with manager:"][..],
                &["with manager:", "for item in items:"][..],
                &["with manager:", "while flag:"][..],
                &["try:"][..],
                &["with manager:", "try:"][..],
                &["try:", "for item in items:"][..],
                &["try:", "try:", "with manager:"][..],
            ] {
                for returned in [
                    "return",
                    "return None",
                    "return True",
                    "return value",
                    "return make(value)",
                    "return value.attr",
                ] {
                    for binding in ["", " as error"] {
                        let mut body = format!(
                            "try:\n    work()\nfinally:\n    try:\n        finish()\n    except Error{binding}:\n        {returned}\n"
                        );
                        for header in headers.iter().rev() {
                            body = format!("{header}\n{}", indent(&body));
                            if *header == "try:" {
                                body.push_str("except Outer:\n    recover()\n");
                            }
                        }
                        let source = format!(
                            "{scope} f(items, value, manager, flag):\n{}",
                            indent(&format!("{prefix}{body}"))
                        );
                        assert_matches_cpython_version(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn retired_iterator_regions_do_not_split_copied_return_footers() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["sync", "async", "generator"] {
            for prefix in [
                "",
                "for value in values:\n    try:\n        value.attr = None\n    except Error:\n        pass\n",
            ] {
                for returned in ["return", "return True", "return result"] {
                    for suffix in ["del values", "finish()"] {
                        let cleanup = format!(
                            "{prefix}if other is not None:\n    try:\n        for value in other:\n            value.attr = None\n    except Error:\n        pass\n{suffix}\n"
                        );
                        let body = format!(
                            "try:\n    {returned}\nexcept Error:\n    recover()\nfinally:\n{}",
                            indent(&cleanup)
                        );
                        let mut body = format!("for item in items:\n{}", indent(&body));
                        if scope == "generator" {
                            body = format!("yield result\n{body}");
                        }
                        let prefix = if scope == "async" { "async " } else { "" };
                        assert_matches_cpython_version(
                            &format!(
                                "{prefix}def f(items, values, other, result):\n{}",
                                indent(&body)
                            ),
                            version,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn active_handlers_own_inner_finalizer_copies_through_managers() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    let oracle = r"
import ast, dis, importlib.util, marshal, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
source = sys.argv[3]
inner = next(node for node in ast.walk(ast.parse(source))
             if isinstance(node, ast.Try) and node.finalbody
             and isinstance(node.body[0], ast.Return))

def cleanup_owner(code):
    function = next(value for value in code.co_consts
                    if isinstance(value, types.CodeType) and value.co_name == 'f')
    instructions = list(dis.get_instructions(function))
    first = next(instruction for instruction in instructions
                 if instruction.positions.lineno == inner.finalbody[0].lineno
                 and instruction.opname != 'NOP')
    owner = next(entry for entry in dis.Bytecode(function).exception_entries
                 if entry.start <= first.offset < entry.end)
    target = next(index for index, instruction in enumerate(instructions)
                  if instruction.offset == owner.target)
    cleanup = [(instruction.opname, instruction.argval)
               for instruction in instructions[target:target + 3]]
    assert owner.lasti
    return owner.depth, owner.lasti, cleanup

expected = cleanup_owner(compile(source, 'legacy_small_exit_boundaries.py', 'exec',
                                 dont_inherit=True, optimize=0))
actual = cleanup_owner(marshal.loads(sys.stdin.buffer.read()))
assert actual == expected, (actual, expected)
";
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let target = version.target();
        let (variable, fallback) = match version {
            PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12"),
            PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13"),
            PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
            PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
        };
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for outer_managers in 0..=2 {
            for inner_managers in 0..=2 {
                for name in ["", " as error"] {
                    for value in ["value", "work(value)"] {
                        for cleanup in [
                            "cleanup(error)\n",
                            "with cleanup_manager():\n    cleanup(error)\n",
                            "with cleanup_manager():\n    error.close()\n",
                        ] {
                            let handler =
                                format!("try:\n    return {value}\nfinally:\n{}", indent(cleanup));
                            let mut body = format!(
                                "try:\n    work(value)\nexcept Error{name}:\n{}",
                                indent(&handler)
                            );
                            for _ in 0..inner_managers {
                                body = format!("with manager():\n{}", indent(&body));
                            }
                            body = format!("try:\n{}finally:\n    finish(value)\n", indent(&body));
                            for _ in 0..outer_managers {
                                body = format!("with manager():\n{}", indent(&body));
                            }
                            let source = format!("def f(value):\n{}", indent(&body));
                            if version <= PythonVersion::Py313 {
                                assert_matches_cpython_version(&source, version);
                                continue;
                            }
                            // Follow this copied cleanup's actual handler target; unrelated
                            // normal-exit locations and borrow choices remain separate.
                            for compiled in [
                                compile_with_options(
                                    &source,
                                    "legacy_small_exit_boundaries.py",
                                    &options,
                                )
                                .unwrap(),
                                compile_bytes_with_options(
                                    source.as_bytes(),
                                    "legacy_small_exit_boundaries.py",
                                    &options,
                                )
                                .unwrap(),
                            ] {
                                let mut child = Command::new(&python)
                                    .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                                    .args([
                                        target.tag.trim_start_matches('v'),
                                        &target.magic_hex(),
                                        &source,
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
                                    .write_all(&compiled.marshal())
                                    .unwrap();
                                let output = child.wait_with_output().unwrap();
                                assert!(
                                    output.status.success(),
                                    "{version:?}: {source}\n{}",
                                    String::from_utf8_lossy(&output.stderr)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn outer_try_owns_finalizers_after_nested_handler_returns() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["sync", "async", "generator"] {
            for outer_name in ["", " as outer_error"] {
                for inner_name in ["", " as inner_error"] {
                    for value in ["True", "value", "work(value)"] {
                        for cleanup in [
                            "cleanup(value)\n",
                            "if flag:\n    cleanup(value)\n",
                            "try:\n    cleanup(value)\nexcept CleanupError:\n    recover(value)\n",
                            "for item in items:\n    cleanup(item)\n",
                        ] {
                            if version == PythonVersion::Py314 && cleanup != "cleanup(value)\n" {
                                continue;
                            }
                            let handler = format!(
                                "try:\n    work(value)\nexcept InnerError{inner_name}:\n    if flag:\n        return {value}\n    recover(value)\n"
                            );
                            let body = format!(
                                "try:\n    work(value)\nexcept Error{outer_name}:\n{}finally:\n{}",
                                indent(&handler),
                                indent(cleanup)
                            );
                            let mut body = format!(
                                "try:\n{}except OuterError:\n    return False\n",
                                indent(&body)
                            );
                            if scope == "generator" {
                                body = format!("yield value\n{body}");
                            }
                            let prefix = if scope == "async" { "async " } else { "" };
                            assert_matches_cpython_version(
                                &format!("{prefix}def f(flag, value, items):\n{}", indent(&body)),
                                version,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn outer_try_owns_each_copied_finalizer_after_a_handler_return() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["sync", "async", "generator"] {
            for handler in ["except Error:", "except Error as error:"] {
                for value in ["True", "value", "work(value)"] {
                    for finalizers in [2, 3] {
                        for outer_tries in [1, 2] {
                            let mut body = format!(
                                "try:\n    work(value)\n{handler}\n    if flag:\n        return {value}\n    recover(value)\nelse:\n    finish(value)\n    return {value}\nfinally:\n    cleanup(value)\n"
                            );
                            for _ in 1..finalizers {
                                body = format!(
                                    "try:\n{}finally:\n    cleanup(value)\n",
                                    indent(&body)
                                );
                            }
                            for _ in 0..outer_tries {
                                body = format!(
                                    "try:\n{}except OuterError:\n    return False\n",
                                    indent(&body)
                                );
                            }
                            if scope == "generator" {
                                body = format!("yield value\n{body}");
                            }
                            let prefix = if scope == "async" { "async " } else { "" };
                            assert_matches_cpython_version(
                                &format!("{prefix}def f(flag, value):\n{}", indent(&body)),
                                version,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn handler_iterator_cleanup_between_finalizers_retains_outer_owner() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["sync", "async", "generator"] {
            for loop_header in ["for item in items:", "while flag:"] {
                for handler in ["except Error:", "except Error as error:"] {
                    for value in ["False", "None", "value", "work(value)"] {
                        for cleanup in [
                            "cleanup(value, flag, value)",
                            "cleanup(value, flag, result=value)",
                        ] {
                            let body = format!(
                                "try:\n    work(value)\n{handler}\n    if flag:\n        return {value}\n    recover(value)\n    break\nfinally:\n    {cleanup}\n"
                            );
                            let body = format!("{loop_header}\n{}", indent(&body));
                            let mut body =
                                format!("try:\n{}finally:\n    finish(value)\n", indent(&body));
                            if scope == "generator" {
                                body = format!("yield value\n{body}");
                            }
                            let prefix = if scope == "async" { "async " } else { "" };
                            assert_matches_cpython_version(
                                &format!("{prefix}def f(flag, value, items):\n{}", indent(&body)),
                                version,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn raising_copied_finalizers_keep_the_surrounding_manager_owner() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, manager) in [
            ("sync", "with"),
            ("async", "with"),
            ("async", "async with"),
            ("generator", "with"),
        ] {
            for value in ["value", "work(value)", "None"] {
                for cleanup in [
                    "try:\n    cleanup(value)\nexcept Error:\n    if flag:\n        raise\n",
                    "failed = work(value) is not None\ntry:\n    if value is not None:\n        cleanup(value)\nexcept Error:\n    if not failed:\n        raise\nfinally:\n    value = None\n",
                ] {
                    let mut body = format!(
                        "try:\n    if flag:\n        return {value}\n    return {value}\nfinally:\n{}",
                        indent(cleanup)
                    );
                    body = format!("{manager} manager():\n{}", indent(&body));
                    body = format!("try:\n{}finally:\n    finish(value)\n", indent(&body));
                    if scope == "generator" {
                        body = format!("yield value\n{body}");
                    }
                    let prefix = if scope == "async" { "async " } else { "" };
                    let source = format!("{prefix}def f(flag, value):\n{}", indent(&body));
                    assert_matches_cpython_version(&source, version);
                }
            }
        }
    }
}

#[test]
fn constant_returns_pop_inner_tries_before_managers_with_outer_finalizers() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, manager) in [
            ("sync", "with"),
            ("async", "with"),
            ("async", "async with"),
            ("generator", "with"),
        ] {
            for value in ["True", "None", "7"] {
                for try_depth in [1, 2] {
                    for cleanup in [
                        "cleanup(value)\n",
                        "if value is not None:\n    cleanup(value)\n",
                        "try:\n    cleanup(value)\nexcept CleanupError:\n    pass\n",
                    ] {
                        let mut body = format!("if flag:\n    return {value}\nwork(value)\n");
                        for _ in 0..try_depth {
                            body = format!(
                                "try:\n{}except Error:\n    recover(value)\n",
                                indent(&body)
                            );
                        }
                        body = format!("{manager} manager():\n{}", indent(&body));
                        body = format!("try:\n{}finally:\n{}", indent(&body), indent(cleanup));
                        if scope == "generator" {
                            body = format!("yield value\n{body}");
                        }
                        let prefix = if scope == "async" { "async " } else { "" };
                        let source = format!("{prefix}def f(flag, value):\n{}", indent(&body));
                        assert_matches_cpython_version(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn copied_finalizers_retire_inner_tries_before_iterator_cleanup() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for iterator in ["for", "async for"] {
            for manager in ["", "with", "async with"] {
                for cleanup in [
                    "cleanup(value)\n",
                    "await cleanup(value)\n",
                    "try:\n    cleanup(value)\nexcept CleanupError:\n    pass\n",
                    "try:\n    await cleanup(value)\nexcept CleanupError:\n    pass\n",
                ] {
                    for value in ["value", "None", "work()"] {
                        let mut body =
                            format!("try:\n    return {value}\nfinally:\n{}", indent(cleanup));
                        if !manager.is_empty() {
                            body = format!("{manager} manager():\n{}", indent(&body));
                        }
                        body = format!("try:\n{}except Error:\n    pass\n", indent(&body));
                        body = format!("{iterator} value in values:\n{}", indent(&body));
                        let source = format!("async def f(values):\n{}", indent(&body));
                        assert_matches_cpython_version(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_conditional_exits_inherit_the_last_opcode_span() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for value in [
            "build(\n            value\n        ).attr",
            "f'{value:.4f}'",
            "f'{value:.4f} units'",
            "f'literal'",
            "'literal'",
            "None",
            "value.attr",
            "call(value)",
        ] {
            let source = format!(
                "def f(flag, other, value):\n    result = (1 if flag else {value}) if other else None\n    return result\n"
            );
            assert_matches_cpython_version(&source, version);
            let source = format!(
                "def f(flag, other, value):\n    assert value == (\n        (1 if flag else {value})\n        if other\n        else None\n    )\n"
            );
            assert_matches_cpython_version(&source, version);
        }
    }
}

#[test]
fn chained_manager_exits_keep_the_only_normal_arm_marker() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for body in [
            "pass",
            "if other:\n    return value",
            "if other is not None:\n    return value",
            "if other or value:\n    return value",
        ] {
            for exit in ["return value", "raise Error(value)"] {
                let source = format!(
                    "def f(manager, flag, other, value):\n    with manager:\n        if flag is not None:\n            {}\n        elif value is not None:\n            {exit}\n        else:\n            {exit}\n",
                    body.replace('\n', "\n            ")
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn nested_else_guard_donors_allow_mixed_earlier_exits() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for first in ["pass", "use(value)", "return value", "raise Error(value)"] {
            for second in ["pass", "use(value)", "return value", "raise Error(value)"] {
                for condition in [
                    "third",
                    "third or value",
                    "third and value",
                    "third is None",
                ] {
                    for terminal in ["return value", "raise Error(value)"] {
                        let source = format!(
                            "def f(outer, first, second, third, value):\n    if outer:\n        if first:\n            {first}\n        elif second:\n            {second}\n        else:\n            if {condition}:\n                {terminal}\n    else:\n        pass\n    finish()\n"
                        );
                        assert_matches_cpython_version(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_while_breaks_leave_the_false_edge_as_the_try_exit_donor() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for condition in ["other", "other or flag", "other and flag"] {
            for body in [
                "pass",
                "use(other)",
                "if flag:\n    break",
                "if flag:\n    use(other)\nelse:\n    break",
                "try:\n    if flag:\n        break\nfinally:\n    finish()",
            ] {
                let source = format!(
                    "def f(items, flag, other):\n    for item in items:\n        try:\n            if flag:\n                pass\n            else:\n                while {condition}:\n                    {}\n        except Error:\n            pass\n        finish()\n",
                    body.replace('\n', "\n                    ")
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn earlier_break_arms_skip_the_normal_try_else_exit() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for async_body in [false, true] {
            for first in ["pass", "use(flag)", "return flag"] {
                for condition in ["flag", "flag or other", "flag and other", "flag is None"] {
                    for nested_else in [false, true] {
                        let (prefix, protected) = if async_body {
                            (
                                "async ",
                                "result = [value for item in items for value in await call(item)]",
                            )
                        } else {
                            ("", "result = call(items)")
                        };
                        let last_arm = if nested_else {
                            format!(
                                "else:\n                other += 1\n                if {condition}:\n                    return"
                            )
                        } else {
                            format!("elif {condition}:\n                return")
                        };
                        let source = format!(
                            "{prefix}def f(items, flag, other):\n    while True:\n        try:\n            {protected}\n        except Error as error:\n            pass\n        else:\n            if flag:\n                {first}\n            elif not other:\n                break\n            {last_arm}\n        if other:\n            pass\n"
                        );
                        assert_matches_cpython_version(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_completed_finalizer_joins_receive_late_fallthrough_locations() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for handler in ["Error", "Error as error"] {
            for (first, second) in [
                ("pass", "pass"),
                ("use(flag)", "use(value)"),
                ("flag = value", "value = flag"),
                ("use(flag)", "items[key] = value"),
            ] {
                let source = format!(
                    "def f(manager, flag, value, items, key):\n    with manager:\n        try:\n            pass\n        except {handler}:\n            pass\n        finally:\n            try:\n                pass\n            finally:\n                if flag is None:\n                    {first}\n                else:\n                    {second}\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn legacy_cold_breaks_keep_the_empty_while_anchor() {
    for condition in ["flag", "flag or other"] {
        let source = format!(
            "def f(flag, other):\n    try:\n        while {condition}:\n            while True:\n                try:\n                    pass\n                except Exception:\n                    break\n            break\n    finally:\n        pass\n"
        );
        assert_matches_cpython_version(&source, PythonVersion::Py313);
    }
}

#[test]
fn conditional_loop_filters_inherit_the_accepted_boolean_leaf() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for body in [
            "value is not None and value.attr == other",
            "value is None or value.attr == other",
            "not value.attr",
        ] {
            for negated in [false, true] {
                let condition = format!(
                    "({body}\n            if other is not None\n            else value == fallback)"
                );
                let condition = if negated {
                    format!("not {condition}")
                } else {
                    condition
                };
                let source = format!(
                    "def f(items, other, fallback):\n    for value in items:\n        if {condition}:\n            use(value)\n"
                );
                assert_matches_cpython_version(&source, version);
                for (open, close) in [("[", "]"), ("(", ")")] {
                    let source = format!(
                        "def f(items, other, fallback):\n    return {open}value for value in items if {condition}{close}\n"
                    );
                    assert_matches_cpython_version(&source, version);
                }
            }
        }
    }
}

#[test]
fn modern_terminal_stores_preserve_the_outer_conditional_footer() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for prefix in ["", "async "] {
            for guard in ["value", "value is not None and value.attr"] {
                let source = format!(
                    "def outer():\n    result = None\n    {prefix}def f(value):\n        nonlocal result\n        if {guard}:\n            result = value if result is None else result.merge(value)\n    return f\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn folded_terminal_conditions_do_not_expose_expression_exits_early() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for prefix in ["", "async "] {
            for expression in [
                "use(value.attr if value is not None else None)",
                "result = value if value else None",
            ] {
                let source = format!(
                    "{prefix}def f(value):\n    {expression}\n    if False:\n        yield None\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn legacy_match_async_exhaustion_keeps_the_case_success_eval_check() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let source = "async def f(manager, items, flag):\n    try:\n        async with manager:\n            async for item in items:\n                match item:\n                    case First(value=value):\n                        pass\n                    case Second():\n                        pass\n                    case Third() | Fourth() | Fifth() as value:\n                        async for output in value:\n                            pass\n                if flag:\n                    pass\n    finally:\n        pass\n";
        assert_matches_cpython_version(source, version);
    }
}

#[test]
fn legacy_handler_returns_and_breaks_count_outer_try_pops() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for condition in ["flag", "flag and value", "value is not None"] {
            for prefix in ["", "async "] {
                let source = format!(
                    "{prefix}def f(value, flag):\n    try:\n        try:\n            work()\n        except ValueError:\n            return value if {condition} else None\n    except Exception:\n        return None\n"
                );
                assert_matches_cpython_version(&source, version);
                let source = format!(
                    "{prefix}def f(value, flag):\n    try:\n        work()\n    finally:\n        while True:\n            try:\n                result = value if {condition} else None\n                break\n            except Exception:\n                pass\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn try_else_returns_keep_the_original_pop_after_branch_nops() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for (prefix, value) in [
            ("async ", "await value"),
            ("", "yield from value"),
            ("", "work()"),
        ] {
            let source = format!(
                "{prefix}def f(value, flag):\n    try:\n        if flag:\n            {value}\n        else:\n            {value}\n    except Exception:\n        pass\n    else:\n        return\n    raise RuntimeError\n"
            );
            assert_matches_cpython_version(&source, version);
        }
    }
}

#[test]
fn conditional_assignments_keep_the_original_shared_store() {
    for target in ["result", "target.value", "target[0]"] {
        for condition in [
            "value",
            "0 < value <= 100",
            "value is not None and 0 < value <= 100",
            "0 < value <= 100 and value is not None",
            "value is None or 0 < value <= 100",
        ] {
            for body in ["value", "value if other else 2"] {
                let assignment = format!("{target} = ({body}) if {condition} else 1");
                assert_matches_cpython(&format!(
                    "def f(target, value, other):\n    {assignment}\n"
                ));
                assert_matches_cpython(&format!(
                    "def outer(target, value, other):\n    result = None\n    def f():\n        nonlocal result\n        {assignment}\n    return f\n"
                ));
            }
        }
    }
}

#[test]
fn conditional_assignment_branches_keep_the_313_shared_return_block() {
    let indent = |source: &str| {
        source.lines().fold(String::new(), |mut output, line| {
            writeln!(output, "    {line}").unwrap();
            output
        })
    };
    for storage in ["local", "global", "nonlocal", "attribute", "subscript"] {
        for shape in ["plain", "if", "else", "elif"] {
            for condition in ["value", "value is not None", "0 < value <= 10"] {
                for value in ["work(value)", "value if other else 2"] {
                    let target = match storage {
                        "attribute" => "target.value",
                        "subscript" => "target[0]",
                        _ => "result",
                    };
                    let assignment = format!("{target} = {value} if {condition} else other\n");
                    let mut body = match shape {
                        "plain" => assignment,
                        "if" => format!("if flag:\n{}", indent(&assignment)),
                        "else" => {
                            format!("if flag:\n{}else:\n    work(value)\n", indent(&assignment))
                        }
                        "elif" => format!(
                            "if flag:\n{}elif other:\n{}else:\n    work(value)\n",
                            indent(&assignment),
                            indent(&assignment)
                        ),
                        _ => unreachable!(),
                    };
                    if matches!(storage, "global" | "nonlocal") {
                        body.insert_str(0, &format!("{storage} result\n"));
                    }
                    let mut source =
                        format!("def f(target, value, flag, other):\n{}", indent(&body));
                    if storage == "nonlocal" {
                        source = format!(
                            "def outer():\n    result = None\n{}    return f\n",
                            indent(&source)
                        );
                    }
                    assert_matches_cpython_version(&source, PythonVersion::Py313);
                }
            }
        }
    }
}

#[test]
fn direct_handler_break_unwinds_tries_before_discarding_the_iterator() {
    let indent = |source: &str| {
        source.lines().fold(String::new(), |mut output, line| {
            writeln!(output, "    {line}").unwrap();
            output
        })
    };
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for asynchronous in [false, true] {
            for generator in [false, true] {
                for named in [false, true] {
                    for managed in [false, true] {
                        for depth in 0..=2 {
                            for finalizer in [false, true] {
                                let mut work = if asynchronous {
                                    "await work(value)\n".to_string()
                                } else {
                                    "work(value)\n".to_string()
                                };
                                if managed {
                                    let prefix = if asynchronous { "async " } else { "" };
                                    work = format!("{prefix}with manager():\n{}", indent(&work));
                                }
                                let name = if named { " as error" } else { "" };
                                let mut handler = format!(
                                    "try:\n{}except Error{name}:\n    break\n",
                                    indent(&work)
                                );
                                if finalizer {
                                    handler.push_str("finally:\n    cleanup(value)\n");
                                }
                                let mut body =
                                    format!("if flag:\n{}work(value)\n", indent(&handler));
                                for _ in 0..depth {
                                    body = format!(
                                        "try:\n{}except Other:\n    continue\n",
                                        indent(&body)
                                    );
                                }
                                body = format!("for value in values:\n{}", indent(&body));
                                if generator {
                                    body.insert_str(0, "yield None\n");
                                }
                                let prefix = if asynchronous { "async " } else { "" };
                                let source = format!(
                                    "{prefix}def f(values, value, flag):\n{}",
                                    indent(&body)
                                );
                                assert_matches_cpython_version(&source, version);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn copied_finally_pop_blocks_survive_both_exit_copy_sweeps() {
    for value in ["None if flag else value", "value if flag else None"] {
        for handler in ["", "    except Exception:\n        raise\n"] {
            for cleanup in ["value = None", "value = 1", "pass"] {
                assert_matches_cpython(&format!(
                    "def f(flag, value):\n    try:\n        return {value}\n{handler}    finally:\n        {cleanup}\n"
                ));
            }
        }
    }
}

#[test]
fn outer_try_pops_follow_copied_finalizers_before_manager_unwind() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for asynchronous in [false, true] {
            let prefix = if asynchronous { "async " } else { "" };
            for cleanup in [
                "for item in items:\n                    finish(item)",
                "if items:\n                    finish(items)",
                "finish(items)",
            ] {
                for value in ["value", "None"] {
                    let source = format!(
                        "{prefix}def f(manager, items, value):\n    {prefix}with manager:\n        try:\n            try:\n                return {value}\n            finally:\n                {cleanup}\n        except Exception:\n            raise\n"
                    );
                    assert_matches_cpython_version(&source, version);
                }
            }
        }
    }
}

#[test]
fn folded_false_tails_retain_original_expression_join_footers() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix) in [
            ("def", ""),
            ("async def", ""),
            ("def", "    yield value\n"),
            ("async def", "    yield value\n"),
        ] {
            for statement in [
                "work(value if flag else None)",
                "work(other, value if flag else None)",
                "work(other, more, value if flag else None)",
                "value.method(other if flag else None)",
                "value.method(more, other if flag else None)",
                "factory()(value if flag else None)",
                "result = value if flag else None",
            ] {
                for discarded in ["yield None", "work()", "pass"] {
                    assert_matches_cpython_version(
                        &format!(
                            "{scope} f(value, other, more, flag):\n{prefix}    {statement}\n    if False:\n        {discarded}\n"
                        ),
                        version,
                    );
                }
            }
        }
    }
}

#[test]
fn shared_finalizer_assertion_exits_inherit_each_success_operand() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, protected) in [
            ("def", "work()\n"),
            ("async def", "await work()\n"),
            ("def", "yield value\nwork()\n"),
            ("async def", "yield value\nwork()\n"),
        ] {
            for condition in [
                "flag",
                "flag or other",
                "flag or not other",
                "value is None or not value.work()",
                "flag or (other and value)",
                "(flag and other) or value",
                "flag if other else value",
            ] {
                for ambient in ["", "with manager:", "try:"] {
                    for follow in ["", "work()\n"] {
                        let mut body = format!(
                            "try:\n{}finally:\n    assert {condition}\n",
                            indent(protected)
                        );
                        if !ambient.is_empty() {
                            body = format!("{ambient}\n{}", indent(&body));
                            if ambient == "try:" {
                                body.push_str("except Error:\n    recover()\n");
                            }
                        }
                        body.push_str(follow);
                        let source =
                            format!("{scope} f(value, flag, other, manager):\n{}", indent(&body));
                        assert_matches_cpython_version(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn finalizer_assertion_exits_inherit_the_unique_success_operand() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for body in ["work()", "return 1"] {
            for condition in [
                "flag and not value.ready",
                "flag and value is None",
                "value is other and not flag",
                "not flag",
            ] {
                let source = format!(
                    "def f(flag, value, other):\n    try:\n        {body}\n    finally:\n        assert {condition}\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn folded_loop_arms_keep_the_313_pre_folding_join_predecessor() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for (prefix, suspension) in [("async ", "await value"), ("", "yield from value")] {
            for first_arm in [
                format!("while True:\n                {suspension}"),
                "return".to_owned(),
                "raise ValueError".to_owned(),
                suspension.to_owned(),
            ] {
                let source = format!(
                    "{prefix}def f(flag, value):\n    try:\n        if flag:\n            {first_arm}\n        else:\n            {suspension}\n    finally:\n        finish()\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn finalizer_returns_unwind_managers_and_iterators_in_stack_order() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for asynchronous in [false, true] {
            let prefix = if asynchronous { "async " } else { "" };
            for value in ["None", "False", "value", "call()"] {
                for nesting in [
                    vec!["for item in items:", "WITH manager:"],
                    vec!["WITH manager:", "for item in items:", "WITH manager:"],
                    vec!["for item in items:", "WITH manager:", "for other in items:"],
                    vec![
                        "for item in items:",
                        "WITH manager:",
                        "for other in items:",
                        "WITH manager:",
                    ],
                ] {
                    let mut source =
                        format!("{prefix}def f(items, manager, flag, value):\n    try:\n");
                    for (index, header) in nesting.iter().enumerate() {
                        writeln!(
                            source,
                            "{}{}",
                            "    ".repeat(index + 2),
                            header.replace("WITH", &format!("{prefix}with"))
                        )
                        .unwrap();
                    }
                    writeln!(
                        source,
                        "{}if flag:\n{}return {value}\n    finally:\n        finish()",
                        "    ".repeat(nesting.len() + 2),
                        "    ".repeat(nesting.len() + 3)
                    )
                    .unwrap();
                    assert_matches_cpython_version(&source, version);
                }
            }
        }
    }
}

#[test]
fn inner_try_pops_precede_manager_exit_with_outer_finalizers() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for outer in [
            "except Exception:\n        raise",
            "finally:\n        finish()",
        ] {
            for value in [
                "flag and value.attr is True",
                "(\n                        await call(\n                            value\n                        )\n                    ).attr",
            ] {
                let source = format!(
                    "async def f(manager, value, flag):\n    try:\n        async with manager, manager:\n            while True:\n                try:\n                    return {value}\n                except Exception:\n                    await value\n    {outer}\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn legacy_terminal_conditionals_keep_call_arm_joins() {
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for condition in ["value", "value is not None", "flag and value"] {
            for asynchronous in [false, true] {
                let prefix = if asynchronous { "async " } else { "" };
                for body in ["value", "str(value or 'fallback')", "1 if flag else 2"] {
                    let source = format!(
                        "def outer(value, flag):\n    result = None\n    {prefix}def f():\n        nonlocal result\n        if flag:\n            result = {body} if {condition} else str(value or 'fallback')\n    return f\n"
                    );
                    assert_matches_cpython_version(&source, version);
                }
            }
        }
    }
}

#[test]
fn handler_store_targets_supply_the_manager_exit_location() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for target in ["result", "value.attr", "value[0]"] {
            for prefix in ["", "async "] {
                let source = format!(
                    "{prefix}def f(manager, value, items):\n    for item in items:\n        with manager:\n            try:\n                work()\n                break\n            except ValueError:\n                raise\n            except Exception as error:\n                {target} = value\n        finish()\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn manager_suppression_exit_copies_keep_unknown_columns() {
    for manager in ["with", "async with"] {
        for result in ["return value", "return None", "return await work()"] {
            for depth in [1, 2] {
                for handler in ["Error", "Error as error"] {
                    let mut cleanup = format!(
                        "try:\n    {manager} manager():\n        await finish()\nexcept {handler}:\n    warning()"
                    );
                    if depth == 2 {
                        cleanup = format!(
                            "try:\n    await cleanup()\nfinally:\n{}",
                            cleanup
                                .lines()
                                .map(|line| format!("    {line}"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        );
                    }
                    let source = format!(
                        "async def process(value):\n    try:\n        {result}\n    finally:\n{}\n",
                        cleanup
                            .lines()
                            .map(|line| format!("        {line}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                    assert_matches_cpython(&source);
                }
            }
        }
    }
}

#[test]
fn chained_guard_loop_exits_use_the_iterable_location() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for target in ["key", "key, value"] {
            let body = format!("for {target} in items:\n            update(target, key)");
            assert_matches_cpython_version(
                &format!(
                    "def process(items, target):\n    if 1 <= len(items) <= 4:\n        {body}\n"
                ),
                version,
            );
            assert_matches_cpython_version(
                &format!(
                    "def process(flag, items, target):\n    if flag:\n        if 1 <= len(items) <= 4:\n            {}\n    elif items:\n        update(target)\n",
                    body.replace('\n', "\n    ")
                ),
                version,
            );
        }
    }
}

#[test]
fn terminating_handler_guards_keep_shared_false_cleanup_unlocated() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for condition in [
            "flag and other",
            "(flag and other) or (value and flag)",
            "flag and not 1 <= value < 4",
            "value and any(item for item in items)",
            "1 < value < 4",
        ] {
            for binding in ["", " as error"] {
                for terminal in ["raise", "return value"] {
                    let source = format!(
                        "async def f(flag, other, value, items):\n    try:\n        work()\n    except Error{binding}:\n        if {condition}:\n            {terminal}\n    finish()\n"
                    );
                    assert_matches_cpython_version(&source, version);
                }
            }
        }
    }
}

#[test]
fn linear_handler_tail_after_nested_branches_keeps_shared_cleanup() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for binding in ["", " as error"] {
            for prefix in [
                "for item in items:\n    pass\nif other:\n    pass",
                "if other:\n    try:\n        work()\n    except Other:\n        pass",
            ] {
                for last in [
                    "value = {}",
                    "value = make()",
                    "consume()",
                    "await consume()",
                ] {
                    let branch = format!("{prefix}\n{last}");
                    let source = format!(
                        "async def f(flag, other, value, items):\n    try:\n        work()\n    except Error{binding}:\n        if flag:\n{}\n    finish()\n",
                        branch
                            .lines()
                            .map(|line| format!("            {line}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                    assert_matches_cpython_version(&source, version);
                }
            }
        }
    }
}

#[test]
fn handler_loop_copies_inherit_locations_before_outer_manager_cleanup() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for outer_manager in [false, true] {
            for outer_try in [false, true] {
                for prefix in ["prepare()", "errors['error'] += 1"] {
                    for loop_header in ["for item in items:", "while flag:"] {
                        for binding in ["", " as error"] {
                            let mut body = format!(
                                "try:\n    work()\nexcept Error{binding}:\n    {prefix}\n    if flag:\n        finish()"
                            );
                            let headers = if outer_manager {
                                ["with manager:", loop_header]
                            } else {
                                [loop_header, "with manager:"]
                            };
                            for header in headers.into_iter().rev() {
                                body = format!(
                                    "{header}\n{}",
                                    body.lines()
                                        .map(|line| format!("    {line}"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                );
                            }
                            if outer_try {
                                body = format!(
                                    "try:\n{}\nexcept Error:\n    pass",
                                    body.lines()
                                        .map(|line| format!("    {line}"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                );
                            }
                            let source = format!(
                                "def f(manager, items, flag, errors):\n{}\n",
                                body.lines()
                                    .map(|line| format!("    {line}"))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            );
                            assert_matches_cpython_version(&source, version);
                        }
                    }
                }
            }
        }
    }
}

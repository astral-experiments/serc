use crate::common;

#[test]
fn matches_cpython_match_or_failure_borrows_after_exit_copying() {
    for pattern in ["1 | 2", "First() | Second()"] {
        for result in [
            "value",
            "value.field",
            "value + 1",
            "(value, 1)",
            "(1, 2)",
            "call(value)",
        ] {
            let source = format!(
                "def f(value):\n    match value:\n        case {pattern}:\n            return {result}\n        case _:\n            use(value)\n"
            );
            common::assert_matches_cpython_marshal(&source, "match_or_failure_borrows.py");
        }
        for body in ["use(value)", "use(value)\n            return value"] {
            let source = format!(
                "def f(value):\n    match value:\n        case {pattern}:\n            {body}\n        case _:\n            use(value)\n"
            );
            common::assert_matches_cpython_marshal(&source, "match_or_failure_fallthrough.py");
        }
    }
}

#[test]
fn matches_cpython_match_or_exit_cost_with_captures_and_later_cases() {
    for pattern in [
        "1 | 2",
        "First() | Second()",
        "[captured, 1] | [captured, 2]",
        "First(value=captured) | Second(value=captured)",
    ] {
        for body in [
            "return None",
            "return value",
            "return value.field",
            "return call(value)",
            "raise Error",
            "use(value)\n            return value",
        ] {
            for later_case in ["", "        case 3:\n            return None\n"] {
                let source = format!(
                    "def f(value):\n    match value:\n        case {pattern}:\n            {body}\n{later_case}        case _:\n            return use(value)\n"
                );
                common::assert_matches_cpython_marshal(&source, "match_or_exit_cost.py");
            }
        }
    }
}

#[test]
fn matches_cpython_match_or_loop_continue_borrows() {
    for pattern in ["1 | 2", "First(value=captured) | Second(value=captured)"] {
        for guard in ["", " if flag"] {
            for later_case in ["", "            case 3:\n                return None\n"] {
                let source = format!(
                    "def f(values, flag):\n    for value in values:\n        match value:\n            case {pattern}{guard}:\n                continue\n{later_case}            case _:\n                return use(value)\n    return values\n"
                );
                common::assert_matches_cpython_marshal(&source, "match_or_loop_continue.py");
            }
        }
    }
}

#[test]
fn matches_cpython_irrefutable_or_exit_size_boundaries() {
    for prefix in ["", "async "] {
        for pattern in ["1 | _", "None | _", "(1 | _) as captured", "1 | 2 | _"] {
            for result in [
                "value",
                "value.attr",
                "value[0]",
                "-value",
                "value + 0",
                "(value, 1)",
                "build(value)",
                "build(**kwargs)",
            ] {
                for guard in ["", " if flag"] {
                    let source = format!(
                        "{prefix}def f(value, build, flag, **kwargs):\n    match value:\n        case {pattern}{guard}:\n            return {result}\n    build(kwargs)\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "irrefutable_or_exit_size.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_irrefutable_or_borrows_inside_cleanup() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield value\n")] {
        for result in ["value", "build(**kwargs)"] {
            let matched = format!(
                "match value:\n    case 1 | _ if flag:\n        return {result}\nbuild(kwargs)"
            );
            for (opening, closing) in [
                ("with manager:", ""),
                ("try:", "\nfinally:\n    build(value)"),
                ("try:", "\nexcept Error:\n    build(value)"),
            ] {
                let protected = format!(
                    "{opening}\n    {}{closing}",
                    matched.replace('\n', "\n    "),
                );
                let source = format!(
                    "{prefix}def f(value, manager, build, flag, **kwargs):\n{leading}    {}\n    build(value)\n",
                    protected.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "protected_irrefutable_or.py");
            }
        }
    }
}

#[test]
fn matches_cpython_local_loads_after_multiline_wildcard_alternatives() {
    // Multiline wildcard patterns still have separate NOP-location differences. Check
    // their local-load instructions and source positions without weakening the strict cases.
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
ours = marshal.loads(pathlib.Path(sys.argv[1]).read_bytes())
reference = compile(pathlib.Path(sys.argv[2]).read_text(), "multiline_wildcard_borrows.py", "exec", dont_inherit=True, optimize=0)
def loads(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    return [(item.opname, item.argval, tuple(item.positions)) for item in dis.get_instructions(code) if item.opname.startswith("LOAD_FAST")]
assert loads(ours) == loads(reference), (loads(ours), loads(reference))
"#;
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield value\n")] {
        for guard in ["", " if flag"] {
            let source = format!(
                "{prefix}def f(value, build, flag, **kwargs):\n{leading}    match value:\n        case (1\n              | _){guard}:\n            return build(value)\n    build(kwargs)\n"
            );
            let directory = tempfile::tempdir().unwrap();
            let source_path = directory.path().join("source.py");
            let marshal_path = directory.path().join("ours.marshal");
            std::fs::write(&source_path, &source).unwrap();
            std::fs::write(
                &marshal_path,
                serc::compile(&source, "multiline_wildcard_borrows.py")
                    .unwrap()
                    .marshal(),
            )
            .unwrap();
            let output = common::python_3145_command()
                .args(["-c", ORACLE])
                .arg(&marshal_path)
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn matches_cpython_match_arm_for_exit_borrow_boundaries() {
    for (filename, source) in [
        (
            "match_or_for_nested_match_borrow_target.py",
            r#"def collect(value):
    match value:
        case {"outer": {"items": [*items]}} | {"items": [*items]}:
            values = []
            for item in items:
                match item:
                    case {"metadata": {"value": str(found)}}:
                        values.append(found)
        case _:
            return None
    unique = set(values)
    if len(unique) != 1:
        return None
    found = next(iter(unique))
    if found not in ALLOWED:
        return None
    return found
"#,
        ),
        (
            "match_or_for_small_return_borrow_control.py",
            r#"def collect(value):
    match value:
        case {"outer": {"items": [*items]}} | {"items": [*items]}:
            values = []
            for item in items:
                match item:
                    case {"metadata": {"value": str(found)}}:
                        values.append(found)
        case _:
            return None
    return values
"#,
        ),
        (
            "match_or_for_post_loop_call_borrow_control.py",
            r#"def collect(value):
    match value:
        case {"outer": {"items": [*items]}} | {"items": [*items]}:
            values = []
            for item in items:
                match item:
                    case {"metadata": {"value": str(found)}}:
                        values.append(found)
            touch(values)
        case _:
            return None
    unique = set(values)
    if len(unique) != 1:
        return None
    found = next(iter(unique))
    if found not in ALLOWED:
        return None
    return found
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrow_aliased_or_pattern_fallback_copy_boundaries() {
    for (success, fallback) in [
        ("value", "raise Error(f'Unsupported: {type(transform)}')"),
        ("value", "return transform"),
        (
            "rank if False else value",
            "raise Error(f'Unsupported: {type(transform)}')",
        ),
        (
            "value",
            "record(transform, rank)\n            return transform if rank else value",
        ),
    ] {
        let source = format!(
            "def f(transform, rank):\n    match transform:\n        case A():\n            return make(transform, rank)\n        case B() | C() as value:\n            return {success}\n        case _:\n            {fallback}\n"
        );
        common::assert_matches_cpython_marshal(&source, "aliased_or_pattern_fallback.py");
    }
}

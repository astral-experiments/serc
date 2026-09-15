use crate::common;

#[test]
fn matches_cpython_match_case_condition_before_common_exit() {
    for condition in ["flag", "flag is not None", "flag and ready"] {
        for exit in ["return subject", "return finish(subject)"] {
            for body in ["consume(flag)", "subject.value = flag"] {
                let source = format!(
                    "def f(subject, flag, ready, consume, fallback, finish):\n    match subject:\n        case 'first':\n            consume(subject)\n            if {condition}:\n                {body}\n        case _:\n            fallback(subject)\n    {exit}\n"
                );
                common::assert_matches_cpython_marshal(&source, "match_case_common_exit.py");
            }
        }
    }
}

#[test]
fn matches_cpython_match_case_condition_before_outer_branch_exit() {
    for looped in [false, true] {
        for condition in ["flag", "flag is not None"] {
            let body = format!(
                "if outer:\n    before(subject)\nelif other:\n    match subject:\n        case 'first':\n            consume(subject)\n            if {condition}:\n                consume(flag)\n        case _:\n            fallback(subject)\nelse:\n    after(subject)\nfinish(subject)\n"
            );
            let body = if looped {
                format!("for subject in subjects:\n{}", indent(&body))
            } else {
                body
            };
            let source = format!(
                "def f(subjects, subject, outer, other, flag):\n{}",
                indent(&body)
            );
            common::assert_matches_cpython_marshal(&source, "match_case_outer_branch_exit.py");
        }
    }
}

#[test]
fn matches_cpython_match_case_terminating_condition_exits() {
    for function in ["def", "async def"] {
        let successors: &[&str] = if function == "def" {
            &[
                "return value",
                "return None",
                "consume(value)",
                "return (value, value, value, value, value)",
            ]
        } else {
            &["return value", "return None"]
        };
        for condition in ["value", "not value", "value and other", "value or other"] {
            for action in ["return None", "return value", "raise Error(value)"] {
                for successor in successors {
                    let source = format!(
                        "{function} f(kind, value, other):\n    match kind:\n        case 'first':\n            if {condition}:\n                {action}\n        case 'second':\n            consume(value)\n        case _:\n            pass\n    {successor}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "match_case_terminating_condition.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_protected_match_wildcard_pass_locations() {
    for (function, generator) in [
        ("def", false),
        ("async def", false),
        ("def", true),
        ("async def", true),
    ] {
        for pattern in ["'first'", "Alpha()", "'first' | 'second'"] {
            for successor in ["consume(value)", "return"] {
                let body = format!(
                    "match kind:\n    case {pattern}:\n        consume(value)\n    case _:\n        pass\n{successor}\n"
                );
                for protected in [
                    format!("try:\n{}except Error:\n    recover()\n", indent(&body)),
                    format!("try:\n    consume(value)\nexcept Error:\n{}", indent(&body)),
                    format!("try:\n    consume(value)\nfinally:\n{}", indent(&body)),
                    format!("with manager:\n{}", indent(&body)),
                ] {
                    let body = if generator {
                        format!("yield value\n{protected}")
                    } else {
                        protected
                    };
                    let source = format!("{function} f(kind, value):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(
                        &source,
                        "protected_match_wildcard_pass.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_match_case_false_guard_bytes() {
    // These terminal handlers have a separate, pre-existing source-position mismatch.
    // Protect their already-correct instruction and exception bytes while that is resolved.
    const ORACLE: &str = r#"
import marshal, pathlib, sys, types
ours = marshal.loads(pathlib.Path(sys.argv[1]).read_bytes())
reference = compile(pathlib.Path(sys.argv[2]).read_text(), "unreachable_wildcard_pass.py", "exec", dont_inherit=True)
ours = next(value for value in ours.co_consts if isinstance(value, types.CodeType))
reference = next(value for value in reference.co_consts if isinstance(value, types.CodeType))
assert ours.co_code == reference.co_code, "instruction bytes differ"
assert ours.co_exceptiontable == reference.co_exceptiontable, "exception table bytes differ"
"#;
    for function in ["def", "async def"] {
        let source = format!(
            "{function} f(kind, value):\n    try:\n        consume(value)\n    except Error:\n        match kind:\n            case 'first':\n                consume(value)\n            case _ if False:\n                pass\n"
        );
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.py");
        let marshal_path = directory.path().join("ours.marshal");
        std::fs::write(&source_path, &source).unwrap();
        std::fs::write(
            &marshal_path,
            serc::compile(&source, "unreachable_wildcard_pass.py")
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

#[test]
fn matches_cpython_last_match_case_conditional_exits() {
    for function in ["def", "async def"] {
        let successors: &[&str] = if function == "def" {
            &[
                "return value",
                "consume(value)",
                "for child in children:\n    consume(child)",
            ]
        } else {
            &["return value"]
        };
        for pattern in ["'last'", "Last()", "First() | Last()"] {
            for condition in ["value", "value and other", "value or other"] {
                for default in ["", "    case _:\n        fallback()\n"] {
                    for successor in successors {
                        let body = format!(
                            "match kind:\n    case 'first':\n        consume(value)\n    case {pattern}:\n        if {condition}:\n            result = consume(value)\n{default}{successor}\n"
                        );
                        let source = format!(
                            "{function} f(kind, value, other, children):\n{}",
                            indent(&body)
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "last_match_conditional_exit.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_outer_branch_match_exit_locations() {
    for function in ["def", "async def"] {
        for pattern in ["'first'", "First()", "First() | Other()"] {
            for default_body in ["consume(value)", "value = produce()"] {
                for case_body in ["consume(value)", "return value"] {
                    for alternate in ["consume(other)", "return other"] {
                        for successor in ["return value", "consume(value)"] {
                            let match_suite = format!(
                                "match kind:\n    case {pattern}:\n        {case_body}\n    case 'second':\n        {case_body}\n    case _:\n        {default_body}\n"
                            );
                            let body = format!(
                                "if outer:\n{}else:\n    {alternate}\n{successor}\n",
                                indent(&match_suite)
                            );
                            let source = format!(
                                "{function} f(kind, value, outer, other):\n{}",
                                indent(&body)
                            );
                            common::assert_matches_cpython_marshal(
                                &source,
                                "outer_branch_match_exit.py",
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_outer_branch_guarded_match_exit_locations() {
    for function in ["def", "async def"] {
        let source = format!(
            "{function} f(kind, value, outer, other):\n    if outer:\n        match kind:\n            case 'first':\n                return value\n            case _ if other:\n                produce(value)\n    else:\n        consume(value)\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "outer_guarded_match_exit.py");
    }
}

#[test]
fn matches_cpython_nested_match_case_shared_exit() {
    for function in ["def", "async def"] {
        for nested in [
            "if flag:\n    consume(value)\n",
            "if ready:\n    if flag:\n        consume(value)\n",
        ] {
            for successor in ["finish(value)\n", "return None\n", "return value\n", ""] {
                for looped in [false, true] {
                    let case_body = format!("if outer:\n{}", indent(nested));
                    let body = format!(
                        "match kind:\n    case 'first':\n{}    case 'other':\n        consume(kind)\n    case _:\n        fallback()\n{successor}",
                        indent(&indent(&case_body))
                    );
                    let body = if looped {
                        format!("for item in items:\n{}", indent(&body))
                    } else {
                        body
                    };
                    let source = format!(
                        "{function} f(kind, outer, ready, flag, value, items, manager):\n{}",
                        indent(&body)
                    );
                    common::assert_matches_cpython_marshal(&source, "nested_match_case_exit.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_match_case_exit_boundaries() {
    for nested in [
        "if flag:\n    consume(value)\nfollowing(value)\n",
        "if flag:\n    consume(value)\nelse:\n    alternative(value)\n",
        "if True:\n    consume(value)\n",
    ] {
        for successor in ["finish(value)\n", "return value\n"] {
            for looped in [false, true] {
                let case_body = format!("if outer:\n{}", indent(nested));
                let body = format!(
                    "match kind:\n    case 'first':\n{}    case 'other':\n        consume(kind)\n    case _:\n        fallback()\n{successor}",
                    indent(&indent(&case_body))
                );
                let body = if looped {
                    format!("for item in items:\n{}", indent(&body))
                } else {
                    body
                };
                let source = format!(
                    "def f(kind, outer, ready, flag, value, items, manager):\n{}",
                    indent(&body)
                );
                common::assert_matches_cpython_marshal(&source, "nested_match_case_boundary.py");
            }
        }
    }
}

#[test]
fn matches_cpython_nested_match_case_predicates() {
    for (function, predicate) in [
        ("def", "flag and ready"),
        ("def", "flag or ready"),
        ("def", "not flag"),
        ("def", "value is None"),
        ("def", "(saved := check(value))"),
        ("async def", "await check(value)"),
        ("def", "flag if ready else outer"),
        ("def", "low < value < high"),
    ] {
        let body = format!(
            "match kind:\n    case \"first\":\n        if outer:\n            if {predicate}:\n                consume(value)\n    case \"other\":\n        consume(kind)\n    case _:\n        fallback()\nfinish(value)\n"
        );
        for body in [
            body.clone(),
            format!("with manager:\n{}", indent(&body)),
            format!("try:\n{}except Error:\n    recover(value)\n", indent(&body)),
        ] {
            let source = format!(
                "{function} f(kind, outer, flag, ready, value, manager, low, high):\n{}",
                indent(&body)
            );
            common::assert_matches_cpython_marshal(&source, "nested_match_case_predicate.py");
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
fn threads_nested_match_case_jump_aliases() {
    for inner in [
        "if first:\n    consume(value)\nelse:\n    before(value)\n    if second:\n        consume(value)\n",
        "if first:\n    consume(value)\nelif second:\n    before(value)\n    if third:\n        consume(value)\n",
    ] {
        for function in ["def", "async def"] {
            let matched = format!(
                "match kind:\n    case 'first':\n{}    case 'other':\n        other(value)\n    case _:\n        fallback(value)\nfinish(value)\n",
                indent(&indent(&format!("if outer:\n{}", indent(inner))))
            );
            for body in [
                matched.clone(),
                format!("with manager:\n{}", indent(&matched)),
                format!(
                    "try:\n{}except Error:\n    recover(value)\n",
                    indent(&matched)
                ),
            ] {
                let source = format!(
                    "{function} f(kind, outer, first, second, third, value, manager):\n{}",
                    indent(&body)
                );
                common::assert_matches_cpython_marshal(&source, "match_case_jump_aliases.py");
            }
        }
    }
}

#[test]
fn preserves_nested_match_case_jump_alias_small_returns() {
    for inner in [
        "if first:\n    consume(value)\nelse:\n    before(value)\n    if second:\n        consume(value)\n",
        "if first:\n    consume(value)\nelif second:\n    before(value)\n    if third:\n        consume(value)\n",
    ] {
        for successor in ["return value", "return None"] {
            let body = format!(
                "match kind:\n    case 'first':\n{}    case 'other':\n        other(value)\n    case _:\n        fallback(value)\n{successor}\n",
                indent(&indent(&format!("if outer:\n{}", indent(inner))))
            );
            let source = format!(
                "def f(kind, outer, first, second, third, value):\n{}",
                indent(&body)
            );
            common::assert_matches_cpython_marshal(&source, "match_case_alias_small_return.py");
        }
    }
}

#[test]
fn matches_cpython_terminal_false_match_guard_locations() {
    for guard in ["False", "0", "None", "''"] {
        let source = format!(
            "def f(value):\n    match value:\n        case 1:\n            consume('first')\n        case _ if {guard}:\n            pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "terminal_false_match_guard_locations.py");
    }
}

#[test]
fn retains_match_elif_success_jump_before_try() {
    for (filename, source) in [
        (
            "match_elif_success_jump_before_try.py",
            r#"def f(value):
    value = value.replace("/", "-")
    if accept(value):
        first, second = value.split()
        if len(first) == 3:
            number = short_names.index(first.lower())
        else:
            number = long_names.index(first.title())
        if check(second) and number:
            value = f"{second}-{number:02d}-01"
    else:
        match len(value):
            case 4:
                value = f"{value}-01-01"
            case 7:
                if check(value[:4]):
                    value = value + "-01"
                elif check(value[-4:]):
                    value = f"{value[-4:]}-{value[0:2]}-01"
            case 8:
                if check(value[-4:]):
                    value = value[0:2] + "-" + value[2:4] + "-" + value[4:]
                elif check(value[:4]):
                    value = value[0:4] + "-" + value[4:6] + "-" + value[6:]
    try:
        parsed = parser.parse(value).isoformat()
        if check(parsed[:4]):
            return parsed
    except Exception as error:
        print(error)
    return ""
"#,
        ),
        (
            "match_else_success_jump_before_try.py",
            r#"def f(value):
    value = value.replace("/", "-")
    if accept(value):
        first, second = value.split()
        if len(first) == 3:
            number = short_names.index(first.lower())
        else:
            number = long_names.index(first.title())
        if check(second) and number:
            value = f"{second}-{number:02d}-01"
    else:
        match len(value):
            case 4:
                value = f"{value}-01-01"
            case 7:
                if check(value[:4]):
                    value = value + "-01"
                elif check(value[-4:]):
                    value = f"{value[-4:]}-{value[0:2]}-01"
            case 8:
                if check(value[-4:]):
                    value = value[0:2] + "-" + value[2:4] + "-" + value[4:]
                else:
                    value = value[0:4] + "-" + value[4:6] + "-" + value[6:]
    try:
        parsed = parser.parse(value).isoformat()
        if check(parsed[:4]):
            return parsed
    except Exception as error:
        print(error)
    return ""
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn copies_name_return_after_final_match_elif() {
    let source = r#"def outer(assignment, meta, block_scope_mergeability):
    def render_block_scope_handle(handle: Handle, handle_has_approval: bool) -> str:
        res = link_to_handle_with_assignment(handle, assignment, meta)
        match handle:
            case UserHandle(_) as user:
                if user not in block_scope_mergeability.approving_users and handle_has_approval:
                    res = f"~{res}~"
            case TeamHandle(_, _) as team:
                if users := block_scope_mergeability.approving_teams.get(team, set()):
                    via = " + ".join(link_to_handle(user) for user in _sorted_handles(users))
                    res = f"{res} _via_ {via}"
                elif handle_has_approval:
                    res = f"~{res}~"
        return res
    return render_block_scope_handle
"#;
    common::assert_matches_cpython_marshal(
        source,
        "match_elif_named_return_after_class_patterns.py",
    );
}

#[test]
fn matches_cpython_final_value_pattern_before_literal_return() {
    for (filename, source) in [
        (
            "match_final_failure_outer_if.py",
            "def f(flag, value):\n    if flag:\n        match value:\n            case 'first':\n                return first()\n            case 'second':\n                return second()\n    else:\n        other(value)\n    return None\n",
        ),
        (
            "match_final_failure_plain_control.py",
            "def f(value):\n    match value:\n        case 'first':\n            return first()\n        case 'second':\n            return second()\n    return None\n",
        ),
        (
            "match_failure_nonempty_continuation_control.py",
            "def f(flag, value):\n    if flag:\n        match value:\n            case 'first':\n                first()\n            case 'second':\n                second()\n        consume(value)\n    else:\n        other(value)\n    return None\n",
        ),
        (
            "match_failure_guarded_control.py",
            "def f(flag, value, ready):\n    if flag:\n        match value:\n            case 'first':\n                return first()\n            case 'second' if ready:\n                return second()\n    else:\n        other(value)\n    return None\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_protected_match_failure_cleanup() {
    for (filename, source) in [
        (
            "protected_match_try_return.py",
            r#"def select(value, first, second, fallback):
    try:
        match value:
            case "first":
                return first()
            case "second":
                return second()
    except ValueError:
        return fallback()
    raise ValueError("unmatched")
"#,
        ),
        (
            "protected_match_with_return.py",
            r#"def select(value, manager, first, second):
    with manager():
        match value:
            case "first":
                return first()
            case "second":
                return second()
    raise ValueError("unmatched")
"#,
        ),
        (
            "protected_match_try_wildcard_control.py",
            r#"def select(value, first, second, fallback):
    try:
        match value:
            case "first":
                return first()
            case "second":
                return second()
            case _:
                return fallback()
    except ValueError:
        return fallback()
    raise ValueError("unmatched")
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

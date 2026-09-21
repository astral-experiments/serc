use std::fmt::Write;

use crate::common;

#[test]
fn preserves_specialized_none_owner_slots_after_source_joins() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", "    await done()\n"),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for prefix in ["", "        scratch = value.attr\n"] {
            for cases in [
                "case None:\n    pass\ncase Other():\n    consume(value)\ncase _:\n    pass\n",
                "case None:\n    pass\ncase _:\n    consume(value)\n",
                "case True:\n    pass\ncase None:\n    consume(value)\ncase _:\n    pass\n",
            ] {
                let mut indented = String::new();
                for line in cases.lines() {
                    writeln!(indented, "            {line}").unwrap();
                }
                let source = format!(
                    "{definition} f(value, enabled):\n    if enabled:\n{prefix}        match value:\n{indented}{tail}",
                );
                common::assert_matches_cpython_marshal(&source, "joined_none_pattern_owners.py");
            }
        }
    }
}

#[test]
fn specializes_none_patterns_in_class_keywords() {
    let indent = |suite: &str| {
        let mut output = String::new();
        for line in suite.lines() {
            writeln!(output, "    {line}").unwrap();
        }
        output
    };
    for kind in ["sync", "async", "generator"] {
        for pattern in [
            "Point(None)",
            "Point(field=None)",
            "Point(field=None, other=None)",
            "Point(field=None as captured)",
        ] {
            for scope in ["plain", "try", "with"] {
                let exits: &[&str] = if scope == "with" {
                    &["return value"]
                } else {
                    &["return value", "consume(value)"]
                };
                for exit in exits {
                    let body = format!(
                        "match value:\n    case {pattern}:\n        {exit}\n    case _:\n        return None\n"
                    );
                    let mut body = match scope {
                        "try" => format!("try:\n{}except Error:\n    return None\n", indent(&body)),
                        "with" => format!("with manager:\n{}", indent(&body)),
                        _ => body,
                    };
                    if kind == "generator" {
                        body.push_str("yield None\n");
                    }
                    let definition = if kind == "async" { "async def" } else { "def" };
                    let source = format!("{definition} f(value, manager):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(&source, "class_keyword_none.py");
                }
            }
        }
    }
}

#[test]
fn specializes_none_patterns_after_protected_comprehensions() {
    let indent = |suite: &str| {
        let mut output = String::new();
        for line in suite.lines() {
            writeln!(output, "    {line}").unwrap();
        }
        output
    };
    for kind in ["sync", "async", "generator"] {
        for pattern in ["None", "1 | None", "None | 1", "True | None"] {
            for scope in ["try_for", "with_for", "finally"] {
                let body = format!(
                    "cached = [item for item in values]\nmatch value:\n    case {pattern}:\n        return value\n    case _:\n        result = 1\n"
                );
                let body = if scope == "finally" {
                    format!("try:\n    process()\nfinally:\n{}", indent(&body))
                } else {
                    let body = format!("for value in values:\n{}", indent(&body));
                    if scope == "with_for" {
                        format!("with manager:\n{}", indent(&body))
                    } else {
                        format!(
                            "try:\n{}except Exception:\n    return False\n",
                            indent(&body)
                        )
                    }
                };
                let definition = if kind == "async" { "async def" } else { "def" };
                let prefix = if kind == "generator" {
                    "    yield None\n"
                } else {
                    ""
                };
                let source = format!(
                    "{definition} f(value, values, manager, process):\n{prefix}{}",
                    indent(&body)
                );
                common::assert_matches_cpython_marshal(&source, "protected_singleton_patterns.py");
            }
        }
    }
}

#[test]
fn preserves_global_call_condition_owner_after_awaiting_match_arm() {
    let source = "async def observe(record, state):\n    match record:\n        case Entry(kind=Mode.START, tool='cache'):\n            await state.begin()\n        case Entry(kind=Mode.STOP, tool='cache'):\n            if isinstance(state.item, Pending) and state.item.status == Status.ACTIVE:\n                await state.item.complete()\n        case _:\n            pass\n";
    common::assert_matches_cpython_marshal(source, "terminal_match_global_call_owner.py");
}

#[test]
fn preserves_global_slice_condition_owner_in_terminal_nested_match() {
    let source = "async def update(event, state):\n    match event:\n        case Entry(kind='start'):\n            await state.begin()\n        case Entry(kind='update'):\n            match state.item:\n                case Item(status=Status.ACTIVE):\n                    piece = event.text[len(state.text):]\n                    if piece:\n                        await state.append(piece)\n                case _:\n                    pass\n        case _:\n            pass\n";
    common::assert_matches_cpython_marshal(source, "terminal_nested_match_slice_owner.py");
}

#[test]
fn preserves_fresh_local_call_marker_after_awaiting_match_arm() {
    let source = "async def observe(record, state, check):\n    match record:\n        case Entry(kind=Mode.START, tool='cache'):\n            await state.begin()\n        case Entry(kind=Mode.STOP, tool='cache'):\n            if check(state.item):\n                await state.item.complete()\n        case _:\n            pass\n";
    common::assert_matches_cpython_marshal(source, "terminal_match_local_call_owner.py");
}

#[test]
fn preserves_global_call_condition_owner_before_any_suspension() {
    let source = "async def observe(record, state):\n    match record:\n        case Entry(kind=Mode.START, tool='cache'):\n            state.begin()\n        case Entry(kind=Mode.STOP, tool='cache'):\n            if isinstance(state.item, Pending):\n                await state.item.complete()\n        case _:\n            pass\n";
    common::assert_matches_cpython_marshal(source, "terminal_match_before_suspension_owner.py");
}

#[test]
fn preserves_value_match_slice_owner_after_fused_attribute_assignment() {
    let source = "async def update(event, state, new):
    match (new, event):
        case (True, Entry(role=Role.ASSISTANT, recipient=recipient)) if recipient == 'user':
            await state.begin()
        case (False, Entry(role=Role.ASSISTANT, recipient=recipient)) if recipient == 'user':
            current = state.item
            assert isinstance(current, Item)
            assert state.part is not None
            status = classify(event)
            match status:
                case Status.ACTIVE:
                    previous = state.part.text
                    content = render(event, 0)
                    state.part.text = content
                    delta = content[len(previous):]
                    if delta:
                        await state.append(delta)
                case Status.COMPLETE | Status.PARTIAL:
                    await state.finish(current)
                    state.part = None
                case _:
                    impossible(status)
";
    common::assert_matches_cpython_marshal(source, "value_match_fused_assignment_slice_owner.py");
}

#[test]
fn preserves_fusion_owner_without_global_call_compaction() {
    let source = "async def update(event, state, render):
    match event.kind:
        case Status.START:
            await state.begin()
        case Status.ACTIVE:
            content = render(event)
            state.part.text = content
            if content:
                await state.append(content)
        case _:
            pass
";
    common::assert_matches_cpython_marshal(source, "value_match_local_fusion_owner.py");
}

#[test]
fn preserves_fresh_manager_singleton_arrays_and_other_identity_tests() {
    for (filename, source) in [
        (
            "match_bool_in_with.py",
            "def f(manager, subject):\n    with manager:\n        match subject:\n            case (False, error):\n                return error\n            case (True, result, name):\n                pass\n            case _:\n                raise ValueError()\n        return result\n",
        ),
        (
            "integer_singleton_manager_control.py",
            "def f(manager, subject):\n    with manager:\n        match subject:\n            case (0, error):\n                return error\n            case (1, result, name):\n                pass\n            case _:\n                raise ValueError()\n        return result\n",
        ),
        (
            "identity_condition_manager_control.py",
            "def f(manager, subject):\n    with manager:\n        if subject is False:\n            return 0\n        elif subject is True:\n            return 1\n        return 2\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_class_singleton_match_break_return_locations() {
    for pattern in ["Point(field=None)", "Point(field=None) | None"] {
        for matched_break in [false, true] {
            let (matched, fallback) = if matched_break {
                ("break", "consume(item)")
            } else {
                ("pass", "break")
            };
            for loop_else in ["", "        else:\n            exhausted()\n"] {
                for outer_else in [false, true] {
                    let mut source = String::from("def f(items, flag):\n    if flag:\n");
                    if outer_else {
                        source.push_str("        alternate()\n    else:\n");
                    }
                    write!(
                        source,
                        "        for item in items:\n            match item:\n                case {pattern}:\n                    {matched}\n                case _:\n                    {fallback}\n{loop_else}"
                    )
                    .unwrap();
                    if !outer_else {
                        source.push_str("    else:\n        alternate()\n");
                    }
                    common::assert_matches_cpython_marshal(&source, "class_pattern_break.py");
                }
            }
        }
    }
}

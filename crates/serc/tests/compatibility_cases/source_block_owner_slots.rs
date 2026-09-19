use crate::common;

#[test]
fn matches_cpython_marshal_for_stored_identity_owner_slots() {
    for asynchronous in ["", "async "] {
        for prefix in [
            "    if flag:\n        return None\n",
            "    value = first if flag else second\n",
        ] {
            for expression in ["value.field is None", "value.field is not None"] {
                for condition in ["selected", "value.field", "value.ready()"] {
                    let source = format!(
                        "{asynchronous}def f(value, first, second, flag, other, load):
{prefix}    selected = {expression}
    try:
        if not {condition}:
            raise Error
    except Error:
        return None
    return selected
"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "stored_identity_owner_slots.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_boolean_comparison_owner_slots() {
    for asynchronous in ["", "async "] {
        for prefix in [
            "    if flag:\n        return None\n",
            "    value = first if flag else second\n",
        ] {
            for expression in [
                "value.field is other.field",
                "not (value.field is other.field)",
                "value.field in other.field",
                "not (value.field in other.field)",
                "value.field not in other.field",
                "not (value.field not in other.field)",
            ] {
                let source = format!(
                    "{asynchronous}def f(value, first, second, flag, other, load, items):
{prefix}    try:
        if {expression}:
            raise Error
    except Error:
        return None
    return value
"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "boolean_comparison_owner_slots.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_conditional_expression_join_owner_slots() {
    for asynchronous in ["", "async "] {
        for expression in [
            "first if flag else second",
            "pick(first) if flag else second",
            "first if flag else pick(second)",
            "(first if flag else second) if other else third",
        ] {
            for condition in ["items", "items.ready", "items.get()"] {
                let source = format!(
                    "{asynchronous}def f(first, second, third, flag, other, items):
    selected = {expression}
    try:
        if not {condition}:
            raise Error
    except Error:
        return None
    return selected
"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "conditional_expression_join_owner_slots.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_post_try_join_owner_slots() {
    for asynchronous in ["", "async "] {
        for prefix in ["", "    if flag:\n        return None\n"] {
            for call in [
                "load(source)",
                "load(source).strip()",
                "global_load(source).strip()",
                "source.strip()",
            ] {
                let source = format!(
                    "{asynchronous}def f(load, source, flag):
{prefix}    try:
        content = {call}
    except Error:
        return None
    if not content:
        return None
    return content
"
                );
                common::assert_matches_cpython_marshal(&source, "post_try_join_owner_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_await_completion_owner_slots() {
    for generator in [false, true] {
        for await_expression in ["await value.close()", "await close(value)", "await run()"] {
            for condition in [
                "(other := value.other)",
                "value.other",
                "(other := flag)",
                "flag",
            ] {
                for finally in [false, true] {
                    let body = if generator { "yield value" } else { "run()" };
                    let prefix = if finally {
                        format!("    try:\n        {body}\n    finally:\n")
                    } else {
                        format!("    {body}\n")
                    };
                    let indent = if finally { "        " } else { "    " };
                    let source = format!(
                        "async def f(value, flag):
{prefix}{indent}{await_expression}
{indent}if {condition}:
{indent}    await finish()
"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "await_completion_owner_slots.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_short_circuit_owner_continuations() {
    for asynchronous in ["", "async "] {
        for generator in [false, true] {
            for context in ["plain", "finally", "with"] {
                for condition in [
                    "value is not None and value.done()",
                    "flag and value.done()",
                    "not flag and not value.done()",
                    "flag or value.done()",
                    "value is not None and value.ready",
                ] {
                    let body = if generator { "yield value" } else { "run()" };
                    let prefix = match context {
                        "finally" => format!("    try:\n        {body}\n    finally:\n"),
                        "with" => format!("    with manager:\n        {body}\n"),
                        _ => format!("    {body}\n"),
                    };
                    let indent = if context == "plain" {
                        "    "
                    } else {
                        "        "
                    };
                    let source = format!(
                        "{asynchronous}def f(value, flag, manager):
{prefix}{indent}if {condition}:
{indent}    finish()
"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "short_circuit_owner_continuations.py",
                    );
                }
            }
        }
    }
}

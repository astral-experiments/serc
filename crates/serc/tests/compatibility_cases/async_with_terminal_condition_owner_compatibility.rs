use crate::common;

#[test]
fn terminal_with_elif_markers_retain_compacted_owners() {
    for (manager, indent) in [
        ("with manager:", "        "),
        ("async with manager:", "        "),
        (
            "async with manager:\n        async with manager:",
            "            ",
        ),
    ] {
        for condition in [
            "isinstance(value, str)",
            "value is not None",
            "work(value)",
            "value",
        ] {
            for body in ["return value", "value = await work()"] {
                let source = format!(
                    r#"async def select(manager, value, work):
    {manager}
{indent}value = await work()
{indent}if value:
{indent}    {body}
{indent}elif {condition}:
{indent}    {body}
{indent}else:
{indent}    return ""
"#
                );
                common::assert_matches_cpython_marshal(&source, "terminal_with_elif_owners.py");
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_async_with_terminal_condition_owners() {
    let source = r#"async def attribute_return(manager, state, pause):
    async with manager:
        if state.first:
            return await pause(state)
        if state.second:
            return
        await pause(state)


async def negated_method_raise(manager, state, pause):
    async with manager:
        if state.is_running():
            return
        if not state.is_paused():
            raise RuntimeError(state)
        await pause(state)


async def and_first_marker(manager, state, event_id, last_poll_id, update):
    async with manager:
        if state.last_event_id and event_id <= state.last_event_id:
            if not last_poll_id:
                state.last_event_id = event_id
            return
        if last_poll_id and event_id <= last_poll_id:
            return
        await update(state)


async def multiple_managers(first, second, state, pause):
    async with first, second:
        if state.first:
            return await pause(state)
        if state.second:
            return
        await pause(state)


async def single_terminal_control(manager, state, pause):
    async with manager:
        if state.second:
            return
        await pause(state)


async def preceding_nonterminal_control(manager, state, pause):
    async with manager:
        if state.first:
            await pause(state)
        if state.second:
            return
        await pause(state)


async def specialized_none_control(manager, state):
    async with manager:
        if state.is_valid():
            return state.token
        if state.tokens["refresh"] is None:
            return None


async def nested_attribute_call_control(manager, read):
    async with manager:
        state = await read()
        if state.value is None:
            raise RuntimeError(state)
        if state.value.is_final():
            state.finish()
            return
"#;
    common::assert_matches_cpython_marshal(source, "async_with_terminal_condition_owners.py");
}

#[test]
fn preserves_pass_markers_inside_protected_async_managers() {
    for suspension in ["", "    yield None\n"] {
        for cleanup in ["except Error:\n    recover()\n", "finally:\n    finish()\n"] {
            for loop_header in ["", "while enabled:", "for item in values:"] {
                for call in ["work()", "await work()"] {
                    let body = format!(
                        "try:\n    async with manager:\n        {call}\n        pass\n{cleanup}"
                    );
                    let body = if loop_header.is_empty() {
                        body
                    } else {
                        format!("{loop_header}\n    {}", body.replace('\n', "\n    "))
                    };
                    let source = format!(
                        "async def process(manager, enabled, values):\n{suspension}    {}\n",
                        body.replace('\n', "\n    ")
                    );
                    common::assert_matches_cpython_marshal(&source, "protected_async_with_pass.py");
                }
            }
        }
    }
}

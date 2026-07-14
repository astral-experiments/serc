#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_coroutine_try_else_attribute_condition_owners() {
    let source = r#"async def attribute(validate, state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.ready:
            observe(state)
    observe("done")


async def not_attribute(validate, state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.ready:
            observe(state)
    observe("done")


async def deep_attribute_two_handlers(validate, state, observe):
    try:
        result = await validate()
    except ValueError:
        observe("invalid")
    except RuntimeError:
        observe("failed")
    else:
        if not state.service.connection.ready:
            observe(result)
    observe("done")


async def nested_input(user_input, validate, state, observe):
    if user_input is not None:
        try:
            await validate(user_input)
        except ValueError:
            observe("invalid")
        else:
            if state.ready:
                observe(state)
    observe("done")


async def maybe_uninitialized_attribute(flag, validate, observe):
    if flag:
        state = make_state()
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.ready:
            observe(state)
    observe("done")


async def deleted_attribute(validate, observe):
    state = make_state()
    del state
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.ready:
            observe("ready")
    observe("done")


async def global_attribute(validate, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if GLOBAL_STATE.ready:
            observe("ready")
    observe("done")


def closure_attribute_factory(state):
    async def closure_attribute(validate, observe):
        try:
            await validate()
        except ValueError:
            observe("invalid")
        else:
            if not state.ready:
                observe(state)
        observe("done")
    return closure_attribute


async def debug_attribute(validate, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if (__debug__).real:
            observe("ready")
    observe("done")


async def not_debug_attribute(validate, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not (__debug__).real:
            observe("ready")
    observe("done")


async def awaited_root_attribute(validate, get_state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if (await get_state()).ready:
            observe("ready")
    observe("done")


async def not_awaited_root_attribute(validate, get_state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not (await get_state()).ready:
            observe("ready")
    observe("done")


async def compare_test(validate, state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.ready is not None:
            observe(state)
    observe("done")


async def handler_await(validate, report, state, observe):
    try:
        validate()
    except ValueError:
        await report()
    else:
        if state.ready:
            observe(state)
    observe("done")


async def terminal_body(validate, state, observe):
    try:
        await validate()
        return state
    except ValueError:
        observe("invalid")
    else:
        if state.ready:
            observe(state)
    observe("done")
"#;
    common::assert_matches_cpython_marshal(
        source,
        "coroutine_try_else_attribute_condition_owners.py",
    );
}

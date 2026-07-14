#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_coroutine_try_else_call_and_subscript_condition_owners() {
    let source = r#"async def method_call(validate, state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.is_ready():
            observe(state)
    observe("done")


async def not_method_call(validate, state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.service.is_ready():
            observe(state)
    observe("done")


async def subscript_name_index(validate, state, key, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.values[key]:
            observe(state)
    observe("done")


async def not_subscript_string_index(validate, state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.values["ready"]:
            observe(state)
    observe("done")


async def subscript_integer_index(validate, state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.values[0]:
            observe(state)
    observe("done")


async def subscript_singleton_indices(validate, state, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.values[None]:
            observe("none")
        if state.values[True]:
            observe("true")
        if state.values[...]:
            observe("ellipsis")
    observe("done")


async def maybe_uninitialized_method(flag, validate, observe):
    if flag:
        state = make_state()
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.is_ready():
            observe(state)
    observe("done")


async def deleted_subscript(key, validate, observe):
    state = make_state()
    del state
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.values[key]:
            observe("ready")
    observe("done")


async def global_method(validate, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if GLOBAL_STATE.service.is_ready():
            observe("ready")
    observe("done")


def closure_subscript_factory(state):
    async def closure_subscript(key, validate, observe):
        try:
            await validate()
        except ValueError:
            observe("invalid")
        else:
            if not state.service.values[key]:
                observe(state)
        observe("done")
    return closure_subscript


async def awaited_positional(validate, state, get_value, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.is_ready(await get_value()):
            observe(state)
    observe("done")


async def awaited_keyword(validate, state, get_value, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.is_ready(value=await get_value()):
            observe(state)
    observe("done")


async def boolean_positional(validate, state, first, second, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.is_ready(first and second):
            observe(state)
    observe("done")


async def boolean_keyword(validate, state, first, second, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.is_ready(value=first or second):
            observe(state)
    observe("done")


async def comprehension_argument(validate, state, values, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.is_ready([item for item in values]):
            observe(state)
    observe("done")


async def awaited_subscript_base(validate, get_state, key, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if (await get_state()).values[key]:
            observe("ready")
    observe("done")


async def awaited_subscript_index(validate, state, get_key, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.values[await get_key()]:
            observe(state)
    observe("done")


async def boolean_subscript_index(validate, state, first, second, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.values[first and second]:
            observe(state)
    observe("done")


async def sliced_subscript(validate, state, start, stop, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if state.values[start:stop]:
            observe(state)
    observe("done")


async def tuple_subscript_index(validate, state, first, second, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if not state.values[first, second]:
            observe(state)
    observe("done")


async def locals_call(validate, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if locals():
            observe("ready")
    observe("done")


async def globals_call(validate, observe):
    try:
        await validate()
    except ValueError:
        observe("invalid")
    else:
        if globals():
            observe("ready")
    observe("done")


class SuperControl:
    async def super_call(self, validate, observe):
        try:
            await validate()
        except ValueError:
            observe("invalid")
        else:
            if super():
                observe(self)
        observe("done")
"#;
    common::assert_matches_cpython_marshal(
        source,
        "coroutine_try_else_call_subscript_condition_owners.py",
    );
}

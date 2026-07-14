use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_boolean_value_owners() {
    let source = r"async def coroutine_one_slot_after_if(value, fallback, run, flag):
    if flag:
        run(flag)
    selected = bool(value or fallback)
    return await run(selected)


async def coroutine_one_slot_after_assert(value, fallback, run, flag):
    assert flag
    selected = bool(value or fallback)
    return await run(selected)


async def coroutine_one_slot_after_bool_condition(value, fallback, run, flag):
    if value and flag:
        run(flag)
    selected = bool(value or fallback)
    return await run(selected)


async def coroutine_one_slot_inside_if(value, fallback, run, flag):
    if flag:
        run(flag)
        selected = bool(value or fallback)
    else:
        selected = fallback
    return await run(selected)


async def coroutine_one_slot_in_raise(value, fallback, run, flag):
    if flag:
        raise RuntimeError(value or fallback)
    return await run(value)


async def coroutine_first_marker_one_slot(value, fallback, run):
    selected = bool(value or fallback)
    return await run(selected)


async def coroutine_first_marker_two_slots(value, fallback, run):
    selected = max(fallback, bool(value or fallback))
    return await run(selected)


async def coroutine_zero_slots_after_if(value, fallback, run, flag):
    if flag:
        run(flag)
    selected = value or fallback
    return await run(selected)


async def coroutine_method_after_if(value, fallback, run, flag, holder):
    if flag:
        run(flag)
    selected = holder.pick(value or fallback)
    return await run(selected)


async def coroutine_three_slots_after_if(value, fallback, run, flag):
    if flag:
        run(flag)
    selected = max(str(fallback), bool(value or fallback))
    return await run(selected)


async def coroutine_three_slots_after_assert(value, fallback, run, flag):
    assert flag
    selected = max(str(fallback), bool(value or fallback))
    return await run(selected)


async def coroutine_try_zero_slot(value, fallback, run, flag):
    try:
        if flag:
            run(flag)
        selected = value or fallback
    except LookupError:
        selected = fallback
    return await run(selected)


async def coroutine_sync_with_zero_slot(value, fallback, run, flag, manager):
    with manager:
        if flag:
            run(flag)
        selected = value or fallback
    return await run(selected)


async def coroutine_after_suspension(value, fallback, run, flag):
    await run(flag)
    if flag:
        run(flag)
    selected = bool(value or fallback)
    return selected


def generator_three_slots_after_if(value, fallback, run, flag):
    if flag:
        run(flag)
    selected = max(str(fallback), bool(value or fallback))
    yield run(selected)


def generator_first_marker_one_slot(value, fallback, run):
    selected = bool(value or fallback)
    yield run(selected)


def plain_one_slot_after_if(value, fallback, run, flag):
    if flag:
        run(flag)
    selected = bool(value or fallback)
    return run(selected)
";
    common::assert_matches_cpython_marshal(source, "coroutine_boolean_value_owners.py");
}

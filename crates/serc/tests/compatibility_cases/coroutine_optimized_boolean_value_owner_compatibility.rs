use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_optimized_boolean_value_owners() {
    let source = r"def sync_control(values, marker, consume):
    return consume(marker, found=any(value.ready for value in values))


def sync_handler_value(values, marker):
    try:
        raise Error
    except Error:
        return marker, any(value.ready for value in values)


def sync_handler_assignment(values, consume):
    try:
        raise Error
    except Error:
        found = all(value.ready for value in values)
        consume(found)


def sync_handler_filtered(values, marker):
    try:
        raise Error
    except Error:
        return marker, any(value.ready for value in values if value.active)


def sync_handler_multiple(groups, marker):
    try:
        raise Error
    except Error:
        return marker, all(value.ready for group in groups for value in group)


def sync_with_handler_control(manager, marker):
    with manager:
        try:
            raise Error
        except Error as error:
            details = error.decode(errors='replace')
            if any(value in details for value in ['first', 'second']):
                consume()
            fail(error, details)


async def coroutine_before(values, marker, consume):
    return consume(marker, found=any(value.ready for value in values))


async def coroutine_after(values, marker, consume, suspend):
    await suspend()
    return consume(marker, found=all(value.ready for value in values))


async def coroutine_assignment_after(values, consume, suspend):
    await suspend()
    found = any(value.ready for value in values)
    consume(found)


async def coroutine_boolean_after(values, marker, suspend):
    await suspend()
    return marker and any(value.ready for value in values)


async def coroutine_for_after(groups, suspend):
    await suspend()
    for values in groups:
        if any(value.ready for value in values):
            break
        await suspend()


async def coroutine_handler_after(values, marker, suspend):
    await suspend()
    try:
        raise Error
    except Error:
        return marker, all(value.ready for value in values)


async def coroutine_first_iter(values):
    return any(value.ready for value in await values())


def generator_value_after(values, marker):
    yield marker
    return marker, any(value.ready for value in values)


def generator_handler_after(values, marker):
    yield marker
    try:
        raise Error
    except Error:
        return marker, all(value.ready for value in values)


def generator_filter_after(values, marker, prepare):
    yield marker
    return any(value.ready for value in prepare(values) if value.active)


async def async_generator_before(values):
    yield any(value.ready for value in values)


def tuple_control(values, marker):
    yield marker
    return tuple(value.ready for value in values)
";
    common::assert_matches_cpython_marshal(source, "coroutine_optimized_boolean_value_owners.py");
}

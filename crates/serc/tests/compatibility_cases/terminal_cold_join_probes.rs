use crate::common;

#[test]
fn terminal_linear_handler_large() {
    common::assert_matches_cpython_marshal(
        r"async def probe(flag, value):
    try:
        value = await read(value)
        if flag:
            return
    except Error as error:
        if error.kind:
            raise
        await recover(value)
    consume(value)
    consume(flag)
    return value
",
        "terminal_linear_handler_large.py",
    );
}

#[test]
fn terminal_nested_handler_large() {
    common::assert_matches_cpython_marshal(
        r"async def probe(flag, value):
    try:
        value = await read(value)
        if flag:
            return
    except Error as error:
        if error.kind:
            raise
        try:
            await recover(value)
        except OtherError as nested:
            if nested.kind:
                raise
    consume(value)
    consume(flag)
    return value
",
        "terminal_nested_handler_large.py",
    );
}

#[test]
fn terminal_nested_handler_shared_condition() {
    common::assert_matches_cpython_marshal(
        r"async def probe(flag, value):
    try:
        value = await read(value)
        if flag and value:
            return
    except Error as error:
        if error.kind:
            raise
        try:
            await recover(value)
        except OtherError as nested:
            if nested.kind:
                raise
    consume(value)
    consume(flag)
    return value
",
        "terminal_nested_handler_shared_condition.py",
    );
}

#[test]
fn terminal_nested_handler_small() {
    common::assert_matches_cpython_marshal(
        r"async def probe(flag, value):
    try:
        value = await read(value)
        if flag:
            return
    except Error as error:
        if error.kind:
            raise
        try:
            await recover(value)
        except OtherError as nested:
            if nested.kind:
                raise
    return value
",
        "terminal_nested_handler_small.py",
    );
}

use crate::common;

#[test]
fn async_manager_surviving_break_followed() {
    common::assert_matches_cpython_marshal(
        r"async def run(manager, value):
    async with manager:
        while True:
            item = await receive()
            if item:
                try:
                    result = parse(item)
                    break
                except Error:
                    report(item)
                    continue
            else:
                report(item)
                continue
        consume(result, value)
        return result
",
        "async_manager_surviving_break_followed.py",
    );
}

#[test]
fn async_manager_direct_break_control() {
    common::assert_matches_cpython_marshal(
        r"async def run(manager, value):
    async with manager:
        while True:
            result = await receive()
            break
        consume(result, value)
        return result
",
        "async_manager_direct_break_control.py",
    );
}

#[test]
fn async_generator_protected_surviving_break_followed() {
    common::assert_matches_cpython_marshal(
        r"async def run(reader, other):
    try:
        while True:
            done = await poll()
            if other is not None and other in done:
                await consume(other.result())
                other = None
            if reader in done:
                item = reader.result()
                if item is None:
                    break
                async for output in handle(item):
                    yield output
                reader = make_reader()
        if other is not None:
            await consume(await other)
        await persist(reader)
    finally:
        if not reader.done():
            reader.cancel()
            await gather(reader)
        if other is not None and not other.done():
            other.cancel()
            await gather(other)
",
        "async_generator_protected_surviving_break_followed.py",
    );
}

#[test]
fn coroutine_protected_surviving_break_control() {
    common::assert_matches_cpython_marshal(
        r"async def run(reader, other):
    try:
        while True:
            done = await poll()
            if other is not None and other in done:
                await consume(other.result())
                other = None
            if reader in done:
                item = reader.result()
                if item is None:
                    break
                async for output in handle(item):
                    consume(output)
                reader = make_reader()
        if other is not None:
            await consume(await other)
        await persist(reader)
    finally:
        if not reader.done():
            reader.cancel()
            await gather(reader)
        if other is not None and not other.done():
            other.cancel()
            await gather(other)
",
        "coroutine_protected_surviving_break_control.py",
    );
}

#[test]
fn outer_for_protected_surviving_break_followed() {
    common::assert_matches_cpython_marshal(
        r"def run(readers, value):
    for reader in readers:
        try:
            while True:
                item = reader.read()
                if item is None:
                    break
                if skip(item):
                    continue
                consume(item)
                if finished(item):
                    break
            consume(reader, value)
        finally:
            reader.close()
",
        "outer_for_protected_surviving_break_followed.py",
    );
}

#[test]
fn outer_for_direct_break_control() {
    common::assert_matches_cpython_marshal(
        r"def run(readers, value):
    for reader in readers:
        try:
            while True:
                item = reader.read()
                break
            consume(reader, value)
        finally:
            reader.close()
",
        "outer_for_direct_break_control.py",
    );
}

#[test]
fn async_generator_folded_following_empty_finally_control() {
    common::assert_matches_cpython_marshal(
        r"async def run(reader, other):
    try:
        while True:
            done = await poll()
            if other is not None and other in done:
                await consume(other.result())
                other = None
            if reader in done:
                item = reader.result()
                if item is None:
                    break
                async for output in handle(item):
                    yield output
                reader = make_reader()
        if False:
            consume(reader)
    finally:
        pass
    consume(reader)
",
        "async_generator_folded_following_empty_finally_control.py",
    );
}

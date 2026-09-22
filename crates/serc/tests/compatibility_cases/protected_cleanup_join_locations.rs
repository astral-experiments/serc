use crate::common;

#[test]
fn matches_cpython_multiple_false_loop_cleanup_joins() {
    for source in [
        r"async def f(first, second, third, work, cleanup):
    try:
        while first and second and third:
            await work()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
",
        r"async def f(first, second, third, work, cleanup):
    try:
        while first and second:
            await work()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
",
        r"async def f(first, second, third, work, cleanup):
    try:
        while first is not None and second is not None:
            await work()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
",
        r"async def f(first, second, third, work, cleanup):
    try:
        while first and second:
            await work()
            pass
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
",
        r"async def f(first, second, third, work, cleanup):
    try:
        while first or second:
            await work()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
",
        r"async def f(first, second, third, work, cleanup):
    try:
        while first and second:
            await work()
        else:
            done()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "protected_cleanup_join_locations.py");
    }
}

#[test]
fn matches_cpython_trailing_while_shared_conditional_cleanup() {
    for source in [
        r"async def f(manager, flag, active, values):
    async with manager:
        try:
            work()
        except Exception as error:
            if flag:
                while active:
                    for value in values:
                        consume(value)
        finally:
            cleanup()
",
        r"def f(manager, flag, active, values):
    with manager:
        try:
            work()
        except Exception as error:
            if flag:
                while active:
                    for value in values:
                        consume(value)
        finally:
            cleanup()
",
        r"async def f(manager, flag, active, values):
    async with manager:
        try:
            work()
        except Exception as error:
            if flag:
                while active:
                    for value in values:
                        consume(value)
            done()
        finally:
            cleanup()
",
        r"async def f(manager, flag, active, values):
    async with manager:
        try:
            work()
        except Exception as error:
            if flag:
                while active:
                    for value in values:
                        consume(value)
            else:
                done()
        finally:
            cleanup()
",
        r"async def f(manager, flag, active, values):
    async with manager:
        try:
            work()
        except Exception as error:
            while active:
                for value in values:
                    consume(value)
        finally:
            cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "protected_cleanup_join_locations.py");
    }
}

#[test]
fn matches_cpython_copied_finalizer_shared_cleanup_joins() {
    for source in [
        r"async def f(manager, outer, session, active, cleanup):
    with manager:
        try:
            async with outer:
                try:
                    while active:
                        try:
                            await work()
                        except Exception:
                            return
                        finally:
                            if session:
                                await close(session)
                                session = None
                finally:
                    finish()
        finally:
            done()
",
        r"async def f(session, cleanup, work):
    try:
        try:
            return
        finally:
            if session is not None:
                await cleanup(session)
                session = None
    finally:
        await work()
",
        r"def f(value):
    try:
        try:
            return
        finally:
            if value is not None:
                value = None
            done()
    finally:
        cleanup()
",
        r"def f(value):
    try:
        try:
            return value
        finally:
            if value:
                raise ValueError
    finally:
        cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "protected_cleanup_join_locations.py");
    }
}

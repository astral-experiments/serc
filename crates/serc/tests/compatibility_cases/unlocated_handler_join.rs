use crate::common;

#[test]
fn matches_cpython_unlocated_handler_joins_after_cleanup() {
    for (filename, source) in [
        (
            "handler_join_after_awaited_finally.py",
            r"async def run(items, manager, send, close, finish):
    for item in items:
        try:
            await send(item)
        except Exception:
            try:
                with manager:
                    await close(item)
            finally:
                finish(item)
",
        ),
        (
            "handler_join_after_manager_return.py",
            r"def read(load, manager, cached):
    try:
        value = load()
    except ValueError:
        with manager:
            return cached
    except Exception:
        with manager:
            return cached
    return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_handler_join_location_controls() {
    for (filename, source) in [
        (
            "located_linear_handler_cleanup.py",
            r"def run(work, recover, finish):
    try:
        work()
    except Exception:
        recover()
    finish()
",
        ),
        (
            "located_handler_cleanup_after_manager.py",
            r"def run(work, manager, cached, finish):
    try:
        work()
    except Exception:
        with manager:
            return cached
        finish()
    return None
",
        ),
        (
            "located_handler_cleanup_after_finally.py",
            r"def run(work, recover, cleanup, finish):
    try:
        work()
    except Exception:
        try:
            recover()
        finally:
            cleanup()
        finish()
    return None
",
        ),
        (
            "named_handler_cleanup_after_manager.py",
            r"def run(work, manager, cached, finish):
    try:
        work()
    except Exception as error:
        with manager:
            return cached
    finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

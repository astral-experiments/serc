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

#[test]
fn matches_cpython_unlocated_conditional_discard_local_return() {
    for (filename, source) in [
        (
            "unlocated_try_conditional_discard_local_return_target.py",
            r"def validate(value):
    try:
        decode(value) if value is not None else None
    except ValueError:
        return None
    return value
",
        ),
        (
            "unlocated_try_conditional_discard_pass_control.py",
            r"def validate(value):
    try:
        decode(value) if value is not None else None
        pass
    except ValueError:
        return None
    return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_named_handler_normal_exit_fallback() {
    for (filename, source) in [
        (
            "nested_named_handler_keyword_fallback_target.py",
            r#"async def nested_handler_fallback(make, work, log, warn, finish):
    try:
        make()
        try:
            await work()
        except Exception as error:
            log(error)
            warn("inner", flush=True)
    except Exception as error:
        log(error)
        warn("outer", flush=True)
    finish()
"#,
        ),
        (
            "nested_named_handler_sync_fallback_control.py",
            r#"def nested_handler_fallback(make, work, log, warn, finish):
    try:
        make()
        try:
            work()
        except Exception as error:
            log(error)
            warn("inner", flush=True)
    except Exception as error:
        log(error)
        warn("outer", flush=True)
    finish()
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

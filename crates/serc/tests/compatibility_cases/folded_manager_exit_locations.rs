use crate::common;

#[test]
fn matches_cpython_marshal_for_manager_folded_loop_exit_locations() {
    for (filename, source) in [
        (
            "sync_manager_folded_loop_finally.py",
            r"def f(manager, work, cleanup):
    try:
        with manager:
            while True:
                work()
    except KeyboardInterrupt:
        return 0
    finally:
        cleanup.close()
",
        ),
        (
            "async_manager_folded_loop_finally.py",
            r#"async def f(manager, fetch, pause, cleanup):
    value = None
    try:
        async with manager:
            while True:
                try:
                    value = await fetch()
                    return value
                except LookupError:
                    await pause()
    except TimeoutError as exc:
        raise ValueError("timeout") from exc
    finally:
        if value is None:
            await cleanup()
"#,
        ),
        (
            "sync_manager_unreachable_loop_control.py",
            r"def f(manager, work, cleanup):
    try:
        raise LookupError
        with manager:
            while True:
                work()
    except KeyboardInterrupt:
        return 0
    finally:
        cleanup.close()
",
        ),
        (
            "sync_manager_authored_pass_join_control.py",
            r"def f(manager, work, cleanup):
    try:
        with manager:
            while True:
                work()
        pass
    except KeyboardInterrupt:
        return 0
    finally:
        cleanup.close()
",
        ),
        (
            "sync_manager_short_finally_control.py",
            r"def f(manager, work):
    try:
        with manager:
            while True:
                work()
    except KeyboardInterrupt:
        return 0
    finally:
        pass
",
        ),
        (
            "sync_manager_break_loop_control.py",
            r"def f(manager, ready, work, cleanup):
    try:
        with manager:
            while True:
                if ready():
                    break
                work()
    except KeyboardInterrupt:
        return 0
    finally:
        cleanup.close()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

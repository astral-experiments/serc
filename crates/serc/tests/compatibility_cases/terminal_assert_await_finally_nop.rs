use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_assert_before_awaited_finally() {
    let cases = [
        (
            "async_named_handler_two_asserts.py",
            r"async def f(run, close, recover):
    result, output = await run()
    try:
        assert result == 0
        assert output == b'ok'
    except Exception as exc:
        recover(exc)
    finally:
        await close()
",
        ),
        (
            "async_unnamed_handler_assert.py",
            r"async def f(value, close, recover):
    try:
        assert value
    except ValueError:
        recover()
    finally:
        await close()
",
        ),
        (
            "async_negated_assert.py",
            r"async def f(predicate, close):
    try:
        assert not predicate()
    except Exception:
        raise
    finally:
        await close()
",
        ),
        (
            "sync_conditional_cleanup.py",
            r"def f(running, stop, recover, parent, child):
    try:
        assert not running(child)
    except ValueError:
        recover()
    finally:
        if running(parent):
            stop(parent)
        if running(child):
            stop(child)
    return None
",
        ),
        (
            "sync_nested_cleanup.py",
            r"def f(rows, cleanup, recover):
    failed = None
    try:
        assert rows == [0, 1, 2]
    except Exception as exc:
        failed = exc
        raise
    finally:
        try:
            cleanup()
        except Exception:
            if failed is None:
                raise
            recover()
    return rows
",
        ),
        (
            "async_without_finally.py",
            r"async def f(value, recover):
    try:
        assert value
    except Exception:
        recover()
",
        ),
        (
            "async_nonterminal_assert.py",
            r"async def f(value, run, close, recover):
    try:
        assert value
        await run()
    except Exception:
        recover()
    finally:
        await close()
",
        ),
        (
            "async_shared_success.py",
            r"async def f(left, right, close):
    try:
        assert left or right
    except Exception:
        raise
    finally:
        await close()
",
        ),
        (
            "async_folded_shared_success.py",
            r"async def f(value, close):
    try:
        assert False or value
    except Exception:
        raise
    finally:
        await close()
",
        ),
        (
            "async_tiny_return_finally.py",
            r"async def f(value):
    try:
        assert value
    except Exception:
        raise
    finally:
        return 0
",
        ),
        (
            "async_pass_finally.py",
            r"async def f(value):
    try:
        assert value
    except Exception:
        raise
    finally:
        pass
",
        ),
        (
            "async_outer_manager.py",
            r"async def f(value, manager, close):
    with manager:
        try:
            assert value
        except Exception:
            raise
        finally:
            await close()
",
        ),
        (
            "async_nested_handler_try.py",
            r"async def f(value, close, recover):
    try:
        run()
    except Exception:
        try:
            assert value
        except ValueError:
            recover()
        finally:
            await close()
",
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_assertion_finally_original_predecessors() {
    let cases = [
        (
            "single_assertion_nested_finally.py",
            r"def f(value, cleanup, recover):
    try:
        assert value
    except Exception:
        raise
    finally:
        try:
            cleanup()
        except Exception:
            recover()
    return value
",
        ),
        (
            "single_async_assertion_nested_finally.py",
            r"async def f(value, cleanup, recover):
    try:
        assert value
    except Exception:
        raise
    finally:
        try:
            await cleanup()
        except Exception:
            recover()
    return value
",
        ),
        (
            "shared_assertion_nested_finally.py",
            r"def f(left, right, cleanup, recover):
    try:
        assert left or right
    except Exception:
        raise
    finally:
        try:
            cleanup()
        except Exception:
            recover()
    return left
",
        ),
        (
            "fallthrough_handler_nested_finally.py",
            r"def f(value, cleanup, recover):
    try:
        assert value
    except Exception:
        recover()
    finally:
        try:
            cleanup()
        except Exception:
            recover()
    return value
",
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

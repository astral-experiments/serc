use crate::common;

#[test]
fn matches_cpython_marshal_for_normal_finally_nonempty_else_entry() {
    let source = r"def target(manager, cleanup, sink):
    result = None
    try:
        with manager:
            pass
    except Exception as error:
        result = error
    else:
        result = True
    finally:
        if cleanup is not None:
            cleanup.close()
        sink(result)


def control(manager, cleanup, sink):
    result = None
    try:
        with manager:
            pass
    except Exception as error:
        result = error
    else:
        result = True
    if cleanup is not None:
        cleanup.close()
    sink(result)
";
    common::assert_matches_cpython_marshal(source, "normal_finally_nonempty_else_entry.py");
}

#[test]
fn matches_cpython_marshal_for_normal_finally_call_first_entry() {
    let source = r#"async def target(manager, session, report):
    try:
        async with manager:
            await session.run()
    except Exception as error:
        try:
            report(error)
        except Exception:
            pass
    finally:
        report("closing")
        await session.close()
    report("done")
"#;
    common::assert_matches_cpython_marshal(source, "normal_finally_call_first_entry.py");
}

#[test]
fn matches_cpython_marshal_for_handler_exits_before_finally_and_name_return() {
    for (source, filename) in [
        (
            r#"async def probe(manager, work, report, clock):
    started = clock()
    status = "failed"
    failure = default_reason
    result = None
    try:
        async with manager:
            result = await work()
    except CancelledError:
        status = "cancelled"
        failure = cancelled_reason
        raise
    except TimeoutError:
        failure = timeout_reason
        report(failure.value)
    except CustomError as error:
        failure = error.reason
        report(failure.value)
    except Exception:
        report(failure.value)
    else:
        status = "success"
        failure = None
    finally:
        reason = failure.value if failure is not None else "none"
        tags = [f"result:{status}", f"reason:{reason}"]
        report(tags=tags)
        report((clock() - started) * 1000, tags=tags)
    return result
"#,
            "async_with_else_finally_handlers.py",
        ),
        (
            r#"async def probe(manager, work, report):
    status = "failed"
    failure = None
    result = None
    try:
        async with manager:
            result = await work()
    except RuntimeError:
        raise
    except Exception:
        report("failed")
    else:
        status = "success"
        failure = None
    finally:
        reason = failure.value if failure is not None else "none"
        report(status, reason)
    return result
"#,
            "async_with_else_finally_minimal.py",
        ),
        (
            r#"async def two_instruction_finally(manager, work, report):
    marker = None
    result = None
    try:
        async with manager:
            result = await work()
    except RuntimeError:
        raise
    except Exception:
        report("failed")
    else:
        marker = "success"
        marker = None
    finally:
        marker = None
    return result


async def call_finally(manager, work, report):
    marker = None
    result = None
    try:
        async with manager:
            result = await work()
    except RuntimeError:
        raise
    except Exception:
        report("failed")
    else:
        marker = "success"
        marker = None
    finally:
        report(marker)
    return result
"#,
            "pending_finally_large_name_return.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_normal_finally_call_after_nested_recovery() {
    let source = r#"async def target(resource, retry):
    try:
        print("starting")
        result = await resource.run()
        print(result)
        print("ready")
    except Exception as error:
        print("failed", error)
        try:
            from math import isfinite as should_retry
        except ImportError:
            should_retry = bool
        if should_retry(retry):
            try:
                print("recovered", resource.recover())
            except Exception:
                print("unavailable")
    finally:
        print("closing")
        await resource.close()
    print("done")
"#;
    common::assert_matches_cpython_marshal(source, "v1_with_post_try_print.py");
}

#[test]
fn matches_cpython_marshal_for_normal_finally_call_with_awaited_local_receiver() {
    let source = r#"async def target(address: str = "https://example.invalid") -> None:
    client = make_client(address=address)
    print("Creating resource...")
    resource = await client.create_resource(kind="local", network=Policy.DEFAULT)
    try:
        print("Preparing the first stage...")
        await resource.worker.execute(["runner", "--script", SETUP_SCRIPT], timeout=6000)
        await resource.worker.execute(["runner", "--script", CONFIG_SCRIPT], timeout=6000)
        print("Running the final stage...")
        result = await resource.worker.execute(["runner", "build", "package"], timeout=180000)
        print(result.decode())
        print("The final stage completed!")
    except Exception as error:
        print("The operation failed:", error)
        try:
            from subprocess import CalledProcessError
        except ImportError:
            CalledProcessError = Exception
        if isinstance(error, CalledProcessError):
            try:
                print("Result:\n", error.result.decode())
            except Exception:
                print("(The result could not be decoded)")
    finally:
        print("Closing resource...")
        await resource.close()
    print("Finished!")
"#;
    common::assert_matches_cpython_marshal(source, "async_local_receiver.py");
}

#[test]
fn matches_cpython_marshal_for_conditional_argument_after_terminal_if_exit() {
    for (source, filename) in [
        (
            r#"def target(flag, work, sink):
    error = None
    try:
        if flag:
            return work()
    except Exception as caught:
        error = caught
    finally:
        sink("failure" if error else "success")
    return error
"#,
            "normal_finally_conditional_join_entry.py",
        ),
        (
            r#"async def target(flag, work, sink):
    error = None
    try:
        if flag:
            return work()
    except Exception as caught:
        error = caught
    finally:
        sink("failure" if error else "success")
    return error
"#,
            "async_conditional_argument_cleanup.py",
        ),
        (
            r"def target(flag, work, sink):
    error = None
    try:
        if flag:
            return work()
    except Exception as caught:
        error = caught
    finally:
        (sink if error else work)(error)
    return error
",
            "conditional_callable_cleanup_control.py",
        ),
        (
            r#"def target(flag, work, sink):
    error = None
    try:
        if flag:
            return work()
    except Exception as caught:
        error = caught
    finally:
        sink(status="failure" if error else "success")
    return error
"#,
            "conditional_argument_keyword_cleanup.py",
        ),
        (
            r"def target(flag, work, sink):
    error = None
    try:
        if flag:
            return work()
    except Exception as caught:
        error = caught
    finally:
        sink(error)
    return error
",
            "normal_finally_linear_argument_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_cell_prologue_and_folded_tuple_assertion() {
    let source = r"from __future__ import annotations


def target() -> None:
    value = make_value((2, 3), dtype)
    interim = make_value((1, 2, 3), dtype)
    result = make_value((1, 2, 2), dtype)
    first_calls: list[tuple[Item, object]] = []
    second_calls: list[tuple[Item, tuple[object, ...]]] = []

    def first(item: Item, shape: object) -> Item:
        first_calls.append((item, shape))
        return interim

    def second(item: Item, *, slices: tuple[object, ...]) -> Item:
        second_calls.append((item, slices))
        return result

    old_first = first_module.convert
    old_second = second_module.select
    first_module.convert = first
    second_module.select = second
    try:
        assert value[None, :, 1:] is result
    finally:
        first_module.convert = old_first
        second_module.select = old_second

    check_results(first_calls, [(value, Shape(1, 2, 3))])
    check_results(
        second_calls,
        [(interim, (slice(None, None, None), slice(None, None, None), slice(1, None, None)))],
    )
";
    common::assert_matches_cpython_marshal(source, "cell_prologue_folded_tuple_assert.py");
}

#[test]
fn matches_cpython_marshal_for_cell_prologue_consuming_local_fusion_slots() {
    for (source, filename) in [
        (
            r"def target():
    value = make_value((1, 2, 3))
    captured = value

    def capture():
        return captured

    first, second = make_pair()
    try:
        assert second == ()
    finally:
        cleanup()
",
            "cell_consumes_one_fusion.py",
        ),
        (
            r"def target():
    value = make_value((1, 2))
    captured = value
    extra = None

    def capture():
        return captured, extra

    first, second = make_pair()
    third, fourth = make_pair()
    try:
        assert fourth == ()
    finally:
        cleanup()
",
            "cells_consume_two_fusions.py",
        ),
        (
            r"def target():
    value = make_value((1, 2, 3))
    captured = value

    def capture():
        return captured

    first = make_pair()
    try:
        assert first == ()
    finally:
        cleanup()
",
            "one_cell_no_fusion_control.py",
        ),
        (
            r"def target():
    value = make_value((1, 2, 3))
    captured = value
    extra = None

    def capture():
        return captured, extra

    first, second = make_pair()
    try:
        assert second == ()
    finally:
        cleanup()
",
            "two_cells_one_fusion_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_loop_finally_owner_assert_owner() {
    let source = r#"async def f(manager, values, stream):
    async with manager:
        try:
            async for value in values:
                pass
        finally:
            assert stream.metadata().get("key", None) == "expected"
            checked = True
    return checked
"#;
    common::assert_matches_cpython_marshal(source, "loop_finally_assert_owner.py");
}

#[test]
fn matches_cpython_marshal_for_cells_consume_fusion_before_null_prefix() {
    let source = r"def target():
    first_value = make_value()
    second_value = make_value(make_value())

    def capture():
        return first_value, second_value

    context = make_context()
    observe()
    first, second = make_pair(make_more())
    try:
        assert second == ()
    finally:
        cleanup()
";
    common::assert_matches_cpython_marshal(source, "cells_consume_fusion_before_null_prefix.py");
}

#[test]
fn matches_cpython_marshal_for_cells_consume_three_fusions_before_null_prefix() {
    let source = r"def target():
    a = make_value((1, 2))
    b = make_value()
    c = make_value()
    d = make_value()
    e = make_value()
    f = make_value()

    def capture():
        return a, b, c, d, e, f

    first, second = make_pair()
    third, fourth = make_pair()
    fifth, sixth = make_pair()
    observe()
    observe()
    observe()
    observe()
    try:
        assert a.wait(timeout=1)
    finally:
        cleanup()
";
    common::assert_matches_cpython_marshal(
        source,
        "cells_consume_three_fusions_before_null_prefix.py",
    );
}

#[test]
fn matches_cpython_marshal_for_large_tuple_cell_assert() {
    let source = r"def target():
    bindings = (1, 2, 3, 4, 5, 6, 7, 8, 9)

    def configure(connection):
        connection.apply(bindings)

    engine = make_engine(configure=configure)
    sessions = make_sessions(engine)
    try:
        result, first_id, second_id = make_result(sessions)
        assert result.attempt_id is not None
        with sessions() as session:
            plan = make_plan(session, result.attempt_id)
        assert plan.ready is True
    finally:
        engine.close()
";
    common::assert_matches_cpython_marshal(source, "large_tuple_cell_assert.py");
}

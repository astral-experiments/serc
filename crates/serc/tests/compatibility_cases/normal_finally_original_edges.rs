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

#[test]
fn matches_cpython_marshal_for_loop_finally_owner_call_owner() {
    let source = r"async def f(manager, values, writer, kind):
    with manager:
        try:
            for value in values:
                await consume(value)
                if value.stop:
                    break
        finally:
            if isinstance(writer, kind):
                writer.close()
            finish()
";
    common::assert_matches_cpython_marshal(source, "loop_finally_call_owner.py");
}

#[test]
fn matches_cpython_marshal_for_retry_terminal_else_borrow() {
    let source = r"async def target(item):
    try:
        value = await load(item)
    except Failure:
        error = explain()
        for attempt in range(2):
            try:
                value = await retry(item)
                yield value
                break
            except Failure:
                error = explain()
        else:
            yield error
            return
    yield pack(item.first, item.second, item.third, item.fourth, item.fifth)
";
    common::assert_matches_cpython_marshal(source, "retry_terminal_else_borrow.py");
}

#[test]
fn matches_cpython_marshal_for_retry_without_else_control() {
    let source = r"async def target(item):
    try:
        value = await load(item)
    except Failure:
        error = explain()
        for attempt in range(2):
            try:
                value = await retry(item)
                yield value
                break
            except Failure:
                error = explain()
    yield pack(item.first, item.second, item.third, item.fourth, item.fifth)
";
    common::assert_matches_cpython_marshal(source, "retry_without_else_control.py");
}

#[test]
fn matches_cpython_marshal_for_loop_finally_owner_cell_none_owner() {
    let source = r"async def f(manager, tasks, progress, writer):
    def capture():
        return progress, writer
    with manager:
        try:
            while tasks:
                await drain(tasks)
        finally:
            progress.close()
            if writer is not None:
                writer.close()
";
    common::assert_matches_cpython_marshal(source, "loop_finally_cell_none_owner.py");
}

#[test]
fn matches_cpython_marshal_for_guarded_finally_keyword_join() {
    common::assert_matches_cpython_marshal(
        r"def guarded(guard, choice, value, other, work, sink):
    try:
        work()
    finally:
        if guard:
            sink(first=value, last=value if choice else other)
",
        "guarded_finally_keyword_join.py",
    );
}

#[test]
fn matches_cpython_marshal_for_guarded_try_assignment() {
    common::assert_matches_cpython_marshal(
        r"def guarded_try(resource, work):
    try:
        work()
    finally:
        if resource is not None:
            try:
                resource.close()
            except Exception:
                pass
            resource = None
",
        "guarded_try_assignment.py",
    );
}

#[test]
fn matches_cpython_marshal_for_unguarded_finally_keyword_join_control() {
    common::assert_matches_cpython_marshal(
        r"def unguarded(guard, choice, value, other, work, sink):
    try:
        work()
    finally:
        sink(first=value, last=value if choice else other)
",
        "unguarded_finally_keyword_join_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_guarded_finally_linear_keyword_control() {
    common::assert_matches_cpython_marshal(
        r"def guarded_linear(guard, choice, value, other, work, sink):
    try:
        work()
    finally:
        if guard:
            sink(first=value, last=other)
",
        "guarded_finally_linear_keyword_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_named_continue_finally_call_owner() {
    let source = r"async def target(items, value, recover):
    for item in items:
        try:
            await work(item)
        except Exception as error:
            if recover:
                first()
                second()
                third()
                fourth()
                continue
            raise
        finally:
            if isinstance(value, Kind):
                value.ready = False
            if isinstance(item, str):
                record(item)
";
    common::assert_matches_cpython_marshal(source, "named_continue_finally_call_owner.py");
}

#[test]
fn matches_cpython_marshal_for_terminal_raise_conditional_finally() {
    let source = r#"def finish_job(start, check, worker_name):
    result = {"error": None}
    worker: object | None = None
    stop_event: object | None = None
    try:
        worker, stop_event = start(worker_name)
        original = worker.settings()
        try:
            worker.configure()
            observed = worker.settings()
        finally:
            worker.restore(original)
        restored = worker.settings()
        if not check(observed, restored):
            raise Error("unexpected state")
    except Error as error:
        result["error"] = format_error(error)
    finally:
        if stop_event is not None:
            stop_event.set()
        if worker is not None:
            worker.join(timeout=5)
            if worker.is_alive():
                previous = result["error"]
                result["error"] = "still running" if previous is None else previous
    result["ok"] = result["error"] is None
    return result
"#;
    common::assert_matches_cpython_marshal(source, "conditional_finally_target.py");
}

#[test]
fn matches_cpython_marshal_for_terminal_raise_early_call_finally() {
    let source = r#"def finish_job(start, check, worker_name):
    result = {"error": None}
    worker: object | None = None
    stop_event: object | None = None
    try:
        worker, stop_event = start(worker_name)
        original = worker.settings()
        try:
            worker.configure()
            observed = worker.settings()
        finally:
            worker.restore(original)
        restored = worker.settings()
        if not check(observed, restored):
            raise Error("unexpected state")
    except Error as error:
        result["error"] = format_error(error)
    finally:
        cleanup_started()
        if stop_event is not None:
            stop_event.set()
        if worker is not None:
            worker.join(timeout=5)
            if worker.is_alive():
                previous = result["error"]
                result["error"] = "still running" if previous is None else previous
    result["ok"] = result["error"] is None
    return result
"#;
    common::assert_matches_cpython_marshal(source, "conditional_finally_early_call_control.py");
}

#[test]
fn matches_cpython_marshal_for_boolean_finally_entry_after_dispatch() {
    let source = r#"def run_job(config, session):
    session.start()
    try:
        wait_ready(session, retry_count=config.retry_count)
        runner = lambda harness, args: run(harness, args, timeout=config.timeout)
        if config.first:
            first(config.harness, first_args(), runner=runner)
        elif config.second:
            second(config.harness, second_args(), runner=runner)
        elif config.third:
            third(config.harness, third_args(), runner=runner)
        else:
            run(config.harness, config.args, runner=runner)
    except Exception as error:
        if config.wait_on_failure:
            report(str(error))
            report("Waiting for interruption")
            try:
                while True:
                    idle()
            except KeyboardInterrupt:
                report("Stopping")
        else:
            raise
    finally:
        if session.started and not config.wait_on_failure:
            session.stop()
    return 0
"#;
    common::assert_matches_cpython_marshal(source, "boolean_finally_dispatch.py");
}

#[test]
fn matches_cpython_marshal_for_single_condition_finally_entry_after_dispatch() {
    let source = r#"def run_job(config, session):
    session.start()
    try:
        wait_ready(session, retry_count=config.retry_count)
        runner = lambda harness, args: run(harness, args, timeout=config.timeout)
        if config.first:
            first(config.harness, first_args(), runner=runner)
        elif config.second:
            second(config.harness, second_args(), runner=runner)
        elif config.third:
            third(config.harness, third_args(), runner=runner)
        else:
            run(config.harness, config.args, runner=runner)
    except Exception as error:
        if config.wait_on_failure:
            report(str(error))
            report("Waiting for interruption")
            try:
                while True:
                    idle()
            except KeyboardInterrupt:
                report("Stopping")
        else:
            raise
    finally:
        if session.started:
            session.stop()
    return 0
"#;
    common::assert_matches_cpython_marshal(source, "single_condition_finally_dispatch.py");
}

#[test]
fn matches_cpython_marshal_for_coroutine_manager_arm_finally_entry() {
    let source = r#"async def prepare(mode, phases, task, measure, tracer, cleanup, record, now):
    try:
        if mode is phases.ACTIVE:
            with measure():
                span = tracer.current()
                token = tracer.token
                if span is not None and token is not None:
                    span.mark(token)
                await task
        else:
            await task
    except InterruptedError:
        record("interrupted")
        await cleanup()
        raise
    except Exception:
        record("failed")
        await cleanup()
        raise
    finally:
        if mode is phases.ACTIVE:
            finished = now()
            record(finished)
"#;
    common::assert_matches_cpython_marshal(source, "coroutine_manager_arm_finally_entry.py");
}

#[test]
fn matches_cpython_marshal_for_coroutine_plain_arm_finally_entry() {
    let source = r#"async def prepare(mode, phases, task, measure, tracer, cleanup, record, now):
    try:
        if mode is phases.ACTIVE:
            await task
        else:
            await task
    except InterruptedError:
        record("interrupted")
        await cleanup()
        raise
    except Exception:
        record("failed")
        await cleanup()
        raise
    finally:
        if mode is phases.ACTIVE:
            finished = now()
            record(finished)
"#;
    common::assert_matches_cpython_marshal(source, "coroutine_plain_arm_finally_entry.py");
}

fn async_else_conditional_finally_source() -> &'static str {
    r#"def make_cleanup(waitable, first_gate, report, second_gate):
    async def cleanup():
        status: str = "failed"
        released = False
        try:
            item = await waitable
        except errors.Cancelled:
            status = "cancelled"
            released = True
        except Exception:
            status = "failed"
            released = True
        else:
            result = await terminate(item, message="cleanup failed")
            if not result.confirmed:
                status = "incomplete"
            else:
                status = "finished"
                released = True
            if result.interrupted:
                raise errors.Cancelled
        finally:
            if released and first_gate is not None:
                first_gate.release()
            try:
                if report is not None:
                    report(status, released)
            finally:
                if released and second_gate is not None:
                    await second_gate.release()
    return cleanup
"#
}

#[test]
fn matches_cpython_marshal_for_async_else_conditional_finally_target() {
    common::assert_matches_cpython_marshal(
        async_else_conditional_finally_source(),
        "async_else_conditional_finally_target.py",
    );
}

#[test]
fn matches_cpython_marshal_for_async_else_conditional_finally_following_call_control() {
    let source = async_else_conditional_finally_source().replace(
        "                raise errors.Cancelled\n        finally:",
        "                raise errors.Cancelled\n            finish_else(status)\n        finally:",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "async_else_conditional_finally_following_call_control.py",
    );
}

fn nonlinear_arm_nested_finally_source() -> &'static str {
    r#"async def finish(state, *, publish=True):
    session = await refresh(state)
    session_key = state.key
    if session is None or session_key is None:
        return False
    durable = False
    info: tuple[str, str] | None = None
    saved = not state.dirty
    try:
        if state.flush:
            await flush(state, force=True)
        elif state.chunk is not None and state.chunk.buffer:
            record("drop", extra={"key": session_key, "value": state.value, "size": len(state.chunk.buffer)})
        info = await upload(state)
        if info is not None:
            committed = await commit(state, info=info)
            durable = committed and available(state)
            if not durable:
                record_blocked(kind="state", reason="commit")
        else:
            durable = available(state)
    except PrimaryError:
        if info is not None:
            record_blocked(kind="state", reason="primary")
        raise
    except errors.Cancelled:
        if info is not None:
            record_blocked(kind="state", reason="cancelled")
        record_error("cancelled", extra={"key": session_key, "value": state.value})
        raise
    except Exception:
        if info is not None:
            record_blocked(kind="state", reason="failure")
        record_error("failure", extra={"key": session_key, "value": state.value})
    finally:
        if state.dirty:
            try:
                await session.save(condition=WriteMode.Unchanged)
                state.dirty = False
                saved = True
            except ConflictError:
                record("conflict", extra={"key": session_key})
                saved = False
            except Exception:
                record_error("save", extra={"key": session_key})
                saved = False
        if saved and durable and state.publish and publish:
            await notify(state)
        state.chunk = None
    return saved and durable
"#
}

#[test]
fn matches_cpython_marshal_for_nonlinear_arm_nested_finally_target() {
    common::assert_matches_cpython_marshal(
        nonlinear_arm_nested_finally_source(),
        "nonlinear_arm_nested_finally_target.py",
    );
}

#[test]
fn matches_cpython_marshal_for_nonlinear_arm_nested_finally_linear_arm_control() {
    let source = nonlinear_arm_nested_finally_source().replace(
        r#"            durable = committed and available(state)
            if not durable:
                record_blocked(kind="state", reason="commit")
"#,
        r#"            durable = merge_commit(committed, state)
            record_blocked(kind="state", reason="commit")
"#,
    );
    common::assert_matches_cpython_marshal(
        &source,
        "nonlinear_arm_nested_finally_linear_arm_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_nonlinear_arm_nested_finally_entry_call_control() {
    let source = nonlinear_arm_nested_finally_source().replace(
        "    finally:\n        if state.dirty:\n",
        "    finally:\n        cleanup_started()\n        if state.dirty:\n",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "nonlinear_arm_nested_finally_entry_call_control.py",
    );
}

fn conditional_handler_guarded_cleanup_source() -> &'static str {
    r#"class Processor:
    def execute(self, key, text, mode, payload):
        worker: object | None = None
        try:
            workspace = self.prepare(key)
            self.journal.record(key, phase="starting")
            session, environment = self.backend.create(mode, key)
            self.journal.record(key, session=session, environment=environment)
            worker = self.backend.start(session, environment, workspace)
            self.journal.record(key, phase="running", identifier=self.backend.identifier(worker))
            self.backend.submit(session, self.render(text, payload))
            terminal = False
            for event in self.backend.events(session, worker):
                self.journal.append(key, event)
                projection = project(event)
                if projection.terminal:
                    terminal = True
                    if not self.action_required(key):
                        self.journal.record(key, phase=projection.phase or "complete", error=projection.error)
                    break
            if not terminal:
                raise RuntimeError("stream ended before completion")
        except ActionRequired as error:
            self.journal.record(key, phase="needs input", error=None, required_action=error.action)
        except Exception as error:
            if not self.action_required(key):
                self.journal.record(key, phase="failed", error=str(error))
        finally:
            if worker is not None:
                try:
                    self.backend.stop(worker)
                except Exception as error:
                    if not self.action_required(key):
                        self.journal.record(key, phase="failed", error=f"cleanup failed: {error}")
"#
}

#[test]
fn matches_cpython_marshal_for_conditional_handler_guarded_cleanup_target() {
    common::assert_matches_cpython_marshal(
        conditional_handler_guarded_cleanup_source(),
        "conditional_handler_guarded_cleanup_target.py",
    );
}

#[test]
fn matches_cpython_marshal_for_conditional_handler_guarded_cleanup_entry_call_control() {
    let source = conditional_handler_guarded_cleanup_source().replace(
        "        finally:\n            if worker is not None:\n",
        "        finally:\n            cleanup_started()\n            if worker is not None:\n",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "conditional_handler_guarded_cleanup_entry_call_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_conditional_handler_guarded_cleanup_body_call_control() {
    let source = conditional_handler_guarded_cleanup_source().replace(
        r#"                raise RuntimeError("stream ended before completion")
        except ActionRequired as error:
"#,
        r#"                raise RuntimeError("stream ended before completion")
            body_finished()
        except ActionRequired as error:
"#,
    );
    common::assert_matches_cpython_marshal(
        &source,
        "conditional_handler_guarded_cleanup_body_call_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_nested_finally_result_write() {
    let source = r#"def worker(datapoint, config, result_path, limit):
    result = {"status": "failed"}
    try:
        manager = nullcontext()
        if limit is not None:
            manager = runtime_context(limit)
        with manager:
            with source_context(config):
                record = datapoint.to_record(config=config)
        payload = serialize(record)
        result = {"status": "completed", "payload": payload}
    except BaseException as error:
        current_error = error
        requires_runtime = False
        while current_error is not None:
            if isinstance(current_error, RuntimeNeeded):
                requires_runtime = True
                break
            current_error = current_error.__cause__ or current_error.__context__
        result = {
            "status": "failed",
            "error_type": type(error).__name__,
            "error": str(error),
            "traceback": traceback.format_exc(),
            "requires_runtime": requires_runtime,
        }
    finally:
        try:
            with open(result_path, "wb") as result_file:
                pickle.dump(result, result_file, protocol=pickle.HIGHEST_PROTOCOL)
        finally:
            release_memory()
"#;
    common::assert_matches_cpython_marshal(source, "normal_nested_finally_result_write_borrow.py");
}

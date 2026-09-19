use crate::common;

#[test]
fn preserves_absent_location_after_folded_with_loop_exit() {
    let source = r"async def f(manager):
    try:
        async with manager:
            while True:
                await step()
    finally:
        await finish()
";
    common::assert_matches_cpython_marshal(source, "finally_async_with_true_loop.py");
}

#[test]
fn retains_location_after_runtime_with_loop_exit() {
    let source = r"async def f(manager, running):
    try:
        async with manager:
            while running():
                await step()
    finally:
        await finish()
";
    common::assert_matches_cpython_marshal(source, "finally_async_with_runtime_loop_control.py");
}

#[test]
fn preserves_absent_location_after_folded_assertion_success() {
    let source = r"async def f(stream):
    try:
        async for value in stream:
            consume(value)
    except Error as error:
        report(error)
        assert False
    finally:
        await finish()
        await close()
";
    common::assert_matches_cpython_marshal(source, "finally_async_for_assert_false.py");
}

#[test]
fn retains_location_without_original_assertion_success() {
    let source = r"async def f(stream):
    try:
        async for value in stream:
            consume(value)
    except Error as error:
        report(error)
        raise Failure
    finally:
        await finish()
        await close()
";
    common::assert_matches_cpython_marshal(source, "finally_async_for_raise_control.py");
}

#[test]
fn shares_while_false_and_break_finally_entry() {
    let source = r"async def f(manager, running):
    async with manager:
        try:
            while running():
                value = await read()
                if value == 'stop':
                    break
                await send(value)
        finally:
            await finish()
            close()
";
    common::assert_matches_cpython_marshal(source, "finally_while_break_under_async_with.py");
}

#[test]
fn distinguishes_copyable_try_exit_from_shared_finally_exit() {
    let source = r"def protected_break(manager, active, stop):
    with manager:
        try:
            while active:
                if stop:
                    break
                step()
        except Error:
            cleanup()

def nested_finally_break(manager, active, stop):
    with manager:
        while True:
            try:
                while active:
                    if stop:
                        break
                    step()
            finally:
                cleanup()
";
    common::assert_matches_cpython_marshal(source, "protected_with_breaks_existing.py");
}

#[test]
fn retains_handler_success_pop_before_nested_finally_try() {
    let source = r"async def target(make, record, attempts):
    for attempt in attempts:
        client = None
        try:
            client = make()
            await client.read()
            record(None)
            return
        except Exception as error:
            retry = bool(attempt)
            record(
                error,
                retry,
            )
        finally:
            try:
                if client is not None:
                    await client.close()
            except Exception as error:
                record(error)
";
    common::assert_matches_cpython_marshal(source, "handler_success_before_nested_finally_try.py");
}

#[test]
fn absorbs_compound_try_exit_into_same_location_finally_pop() {
    for (filename, source) in [
        (
            "compound_try_return_else_before_conditional_finally.py",
            r"async def probe(request, close_sessions, made_sessions):
    token = ''
    url = ''
    try:
        response = await request()
        if response is None:
            return Output(output_dict={})
        else:
            token = response.get('token')
            url = response.get('url')
    except Exception as error:
        return Output(output_dict={'success': False, 'error': str(error)})
    finally:
        if made_sessions:
            await close_sessions()
    return token, url
",
        ),
        (
            "compound_try_return_then_assign_finally_control.py",
            r"async def probe(request, close_sessions, made_sessions):
    token = ''
    url = ''
    try:
        response = await request()
        if response is None:
            return Output(output_dict={})
        token = response.get('token')
        url = response.get('url')
    except Exception as error:
        return Output(output_dict={'success': False, 'error': str(error)})
    finally:
        if made_sessions:
            await close_sessions()
    return token, url
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_else_while_normal_exit_location() {
    let source = r"def probe(local_workers, indices, in_flight, process_piece, consume_result, commit_progress):
    executor = None
    rows_seen = 0
    rows_skipped = 0
    committed_idx = None

    def submit_next():
        return schedule(executor, in_flight)

    try:
        if local_workers <= 1:
            for i in indices:
                rows_seen += 1
                if committed_idx is not None and i <= committed_idx:
                    rows_skipped += 1
                    continue
                consume_result(process_piece(i))
        else:
            executor = Pool(max_workers=local_workers)
            for _ in range(local_workers * 2):
                if not submit_next():
                    break
            while in_flight:
                future = in_flight.popleft()
                consume_result(future.result())
                submit_next()
    except KeyboardInterrupt:
        warn()
        commit_progress()
        raise
    finally:
        if executor is not None:
            executor.shutdown(wait=True, cancel_futures=True)
        commit_progress()
        if rows_skipped:
            report(rows_seen, rows_skipped)
    return iter(())
";
    common::assert_matches_cpython_marshal(source, "protected_final_else_while_exit_location.py");
    let source = r"def probe(local_workers, indices, in_flight, process_piece, consume_result, commit_progress):
    executor = None
    rows_seen = 0
    rows_skipped = 0
    committed_idx = None

    def submit_next():
        return schedule(executor, in_flight)

    try:
        executor = Pool(max_workers=local_workers)
        for _ in range(local_workers * 2):
            if not submit_next():
                break
        while in_flight:
            future = in_flight.popleft()
            consume_result(future.result())
            submit_next()
    except KeyboardInterrupt:
        warn()
        commit_progress()
        raise
    finally:
        if executor is not None:
            executor.shutdown(wait=True, cancel_futures=True)
        commit_progress()
        if rows_skipped:
            report(rows_seen, rows_skipped)
    return iter(())
";
    common::assert_matches_cpython_marshal(
        source,
        "protected_direct_while_exit_location_control.py",
    );
}

#[test]
fn matches_cpython_conditional_finally_before_split_tail_if() {
    for (filename, source) in [
        (
            "conditional_finally_sync_target.py",
            r"def retry():
    while active():
        try:
            work()
            return
        except Error:
            recover()
        finally:
            if resource is not None:
                close(resource)
        if stop():
            return
",
        ),
        (
            "conditional_finally_async_target.py",
            r"async def reconnect():
    while running():
        try:
            await work()
        except Error:
            recover()
        finally:
            if connected():
                await close()
                state.connected = False
        if running():
            await pause()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_conditional_finally_tail_boundaries() {
    for (filename, source) in [
        (
            "conditional_finally_direct_backedge_control.py",
            r"def retry():
    while active():
        try:
            work()
            return
        except Error:
            recover()
        finally:
            if resource is not None:
                close(resource)
",
        ),
        (
            "conditional_finally_folded_true_control.py",
            r"def retry():
    while active():
        try:
            work()
            return
        except Error:
            recover()
        finally:
            if resource is not None:
                close(resource)
        if True:
            return
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_absent_location_after_generator_else_loop_break() {
    let source = r"def f(value, work, ready, cleanup):
    def finish():
        cleanup(value)
    try:
        work()
        yield value
    except BaseException:
        work()
        raise
    else:
        work()
        while True:
            work()
            if ready():
                work()
                break
            work()
    finally:
        finish()
";
    common::assert_matches_cpython_marshal(source, "generator_else_while_break_local_finally.py");
}

#[test]
fn retains_normal_finally_pop_before_nested_try_after_structured_cleanup() {
    for (filename, source) in [
        (
            "normal_finally_nested_try_after_async_generator_cleanup.py",
            r"async def target(stream, cleanup, finish):
    failure = None
    try:
        try:
            async for item in stream:
                yield item
        finally:
            cleanup()
    except BaseException as exc:
        failure = exc
        raise
    finally:
        try:
            finish(failure)
        except Exception:
            pass
",
        ),
        (
            "normal_finally_nested_try_after_sync_manager_return.py",
            r"def target(manager, consume, finish):
    try:
        with manager as value:
            consume(value)
            return True, None
    except Exception as exc:
        return False, exc
    finally:
        try:
            finish()
        except Exception:
            pass
",
        ),
        (
            "normal_finally_nested_try_after_async_manager_return.py",
            r"async def target(manager, work, finish):
    try:
        async with manager:
            value = await work()
            return value
    except TimeoutError:
        return None
    except RuntimeError:
        raise
    except Exception:
        return None
    finally:
        try:
            finish()
        except Exception:
            pass
",
        ),
        (
            "normal_finally_linear_cleanup_after_manager_return_control.py",
            r"def target(manager, consume, finish):
    try:
        with manager as value:
            consume(value)
            return True, None
    except Exception as exc:
        return False, exc
    finally:
        finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_normal_finally_cleanup_donor_locations() {
    for (filename, source) in [
        (
            "normal_finally_nested_try_named_handler_donor_target.py",
            r"def exercise(items, work, recover, stop, cleanup):
    result = None
    for item in items:
        try:
            result = work(item)
            break
        except RuntimeError as error:
            if stop(item):
                break
            result = recover(error)
        finally:
            try:
                cleanup()
            except ValueError:
                pass
    return result
",
        ),
        (
            "normal_finally_direct_named_handler_donor_control.py",
            r"def exercise(items, work, recover, stop, cleanup):
    result = None
    for item in items:
        try:
            result = work(item)
            break
        except RuntimeError as error:
            if stop(item):
                break
            result = recover(error)
        finally:
            cleanup()
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

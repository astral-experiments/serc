use crate::common;

#[test]
fn matches_cpython_borrows_after_shared_conditional_try_exits() {
    for prefix in ["", "async "] {
        for condition in ["flag", "flag and value", "flag or value"] {
            for returned in ["value", "value.attr"] {
                let source = format!(
                    "{prefix}def probe(values, flag, work, use):\n    for item in values:\n        try:\n            value = work(item)\n            if {condition}:\n                return {returned}\n        except Exception:\n            pass\n        use(item)\n"
                );
                common::assert_matches_cpython_marshal(&source, "shared_conditional_try_exit.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_before_extracted_try_loop_tails() {
    for (source, filename) in [
        (
            r"def probe(items, canonical_left, canonical_right):
    for left, right in items:
        try:
            if left != canonical_left:
                left.unlink(missing_ok=True)
        except Exception:
            pass
        try:
            if right != canonical_right:
                right.unlink(missing_ok=True)
        except Exception:
            pass
",
            "consecutive_try_conditional_loop.py",
        ),
        (
            r"def probe(rows, output):
    total = 0
    for left, right in rows:
        try:
            if left is not None:
                total += int(float(left))
        except ValueError:
            pass
        try:
            if right is not None:
                output.append(float(right))
        except ValueError:
            pass
    return total
",
            "consecutive_try_none_guard_loop.py",
        ),
        (
            r"def probe(items, canonical_left, canonical_right):
    for left, right in items:
        try:
            if left != canonical_left:
                left.unlink(missing_ok=True)
            canonical_left = left
        except Exception:
            pass
        try:
            if right != canonical_right:
                right.unlink(missing_ok=True)
        except Exception:
            pass
",
            "consecutive_try_populated_exit_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn borrows_through_nested_recovery_branches_before_raising_handlers() {
    for (source, filename) in [
        (
            r"def f(flag, source, consume):
    try:
        try:
            if flag:
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
    except Exception as exc:
        consume(exc)
        raise
    return consume(value)
",
            "01_sync_nested_recovery_outer_raise.py",
        ),
        (
            r"async def f(flag, source, consume):
    try:
        try:
            if flag:
                value = await source.first()
            else:
                value = await source.second()
        except ValueError as exc:
            raise RuntimeError(exc) from exc
        except Exception as exc:
            if flag:
                raise
            value = source.fallback(exc)
            consume(value)
    except Exception:
        consume(source)
        raise
    return consume(value, source)
",
            "02_async_nested_recovery_outer_raise.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_nested_recovery_borrow_entry_boundaries() {
    for (source, filename) in [
        (
            r"def f(source, consume):
    try:
        try:
            value = source.first()
        except ValueError:
            value = source.fallback()
    except Exception as exc:
        consume(exc)
        raise
    return consume(value)
",
            "03_linear_nested_recovery_control.py",
        ),
        (
            r"def f(flag, source, consume):
    try:
        try:
            if flag:
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
        consume(value)
    except Exception as exc:
        consume(exc)
        raise
    return consume(value)
",
            "05_outer_suffix_control.py",
        ),
        (
            r"def f(flag, source, consume):
    try:
        try:
            if flag:
                while True:
                    pass
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
    except Exception:
        consume(source)
        raise
    return consume(value)
",
            "07_infinite_arm_dead_store_control.py",
        ),
        (
            r"class C:
    try:
        try:
            if flag:
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
    except Exception:
        consume(source)
        raise
    consume(value)
",
            "08_class_scope_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_guarded_terminal_loop_exits() {
    for (source, filename) in [
        (
            r"def f(data, check, load, report):
    value = None
    try:
        value = load(data)
        report(value)
        if data and check(data):
            report(data)
            while True:
                report(value)
                try:
                    value = load(data)
                except Exception as error:
                    report(error)
                    continue
                report(value)
                if check(value):
                    report(data)
                    break
    except Exception as error:
        report(error)
    result = load(data)
    report(value, result)
    if check(result):
        result = load(value)
    report(data, result)
    return result
",
            "compound_guard_terminal_while_break_target.py",
        ),
        (
            r#"def f(data, check, load, report):
    result = None
    try:
        entries = load(data)
        report(entries)
        if data and check(data):
            for entry in entries:
                try:
                    row = load(entry)
                except Exception:
                    continue
                if check(row):
                    if row.ready:
                        return {"row": row, "data": data, "ready": True}
                    result = load(row)
                    if result.ready:
                        result.value = data
                        return result
                    break
    except Exception:
        pass
    result = load(data)
    report(data, result)
    if check(result):
        result = load(result)
    report(data, result)
    return result
"#,
            "compound_guard_terminal_for_break_target.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_guarded_assertion_recovery() {
    for (source, filename) in [
        (
            r#"def f(data, check, load, report):
    result = load(data)
    try:
        report(result)
        if data and check(data):
            assert result.ready, "not ready"
            try:
                assert result.value == {} or result.value is None
                result.value = {}
            except Exception:
                pass
    except (AssertionError, ValueError, TypeError):
        report(data, result)
        return []
    answers = []
    for entry in data.get("entries", []):
        answers.append((entry, load(entry), result))
    report(data, answers)
    if check(answers):
        answers.append(result)
    return answers
"#,
            "compound_guard_assert_recovery_target.py",
        ),
        (
            r#"def f(data, check, load, report):
    result = load(data)
    try:
        report(result)
        if data:
            assert result.ready, "not ready"
            try:
                assert result.value == {} or result.value is None
                result.value = {}
            except Exception:
                pass
    except (AssertionError, ValueError, TypeError):
        report(data, result)
        return []
    answers = []
    for entry in data.get("entries", []):
        answers.append((entry, load(entry), result))
    report(data, answers)
    if check(answers):
        answers.append(result)
    return answers
"#,
            "compound_guard_assert_recovery_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_nested_first_arm_handler_exit() {
    for (source, filename) in [
        (
            r"async def f(context, choose, load, report):
    result = None
    try:
        value = await load(context)
        result = await load(context, value=value)
    except Exception as error:
        if not await choose(context):
            status = await load(context)
            if status and context.active:
                report(context, status)
                value = None
                result = load(context)
            else:
                report(context, status)
                raise error
        else:
            report(context)
            value = None
            result = load(context)
    enabled = choose(context)
    status = await load(context, value=value)
    result.status = status
    if enabled:
        result.value = await load(context)
    report(context, result)
    return result
",
            "nested_first_arm_handler_exit_target.py",
        ),
        (
            r"async def f(context, choose, load, report):
    result = None
    try:
        value = await load(context)
        result = await load(context, value=value)
    except Exception as error:
        if not await choose(context):
            status = await load(context)
            if status and context.active:
                report(context, status)
                value = None
                result = load(context)
            else:
                report(context, status)
                result = load(context)
        else:
            report(context)
            value = None
            result = load(context)
    enabled = choose(context)
    status = await load(context, value=value)
    result.status = status
    if enabled:
        result.value = await load(context)
    report(context, result)
    return result
",
            "nested_first_arm_handler_exit_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_protected_async_elif_breaks() {
    for (source, filename) in [
        (
            r"async def collect(stream, decode, handle, close, report):
    try:
        async for message in stream:
            if message.kind == 1:
                try:
                    payload = decode(message.data)
                except ValueError:
                    continue
                await handle(payload)
            elif message.kind == 2:
                break
    finally:
        report(stream)
        await close(stream)
    report(stream)
    return stream
",
            "async_terminal_elif_retry_arm_target.py",
        ),
        (
            r"async def collect(stream, decode, handle, close, report):
    try:
        async for message in stream:
            if message.kind == 1:
                payload = decode(message.data)
                await handle(payload)
            elif message.kind == 2:
                break
    finally:
        report(stream)
        await close(stream)
    report(stream)
    return stream
",
            "async_terminal_elif_linear_arm_control.py",
        ),
        (
            r#"async def collect(stream, manager, close, report):
    result = None
    values = []
    try:
        async with manager:
            async for message in stream:
                if message.kind == 0:
                    result = message.value
                elif message.kind == 1:
                    values.append(message.value)
                elif message.kind == 2:
                    raise ValueError(message.code or message.name)
                elif message.kind == 3:
                    result = message.value
                    if message.failed:
                        raise ValueError(result)
                    break
    finally:
        await close(stream)
    if result is None:
        raise ValueError("no result")
    report(stream, result, values)
    return result, values
"#,
            "async_terminal_elif_raising_prefix_target.py",
        ),
        (
            r#"async def collect(stream, manager, close, report):
    result = None
    values = []
    try:
        async with manager:
            async for message in stream:
                if message.kind == 0:
                    result = message.value
                elif message.kind == 1:
                    values.append(message.value)
                elif message.kind == 2:
                    raise ValueError(message.name)
                elif message.kind == 3:
                    result = message.value
                    report(result)
                    break
    finally:
        await close(stream)
    if result is None:
        raise ValueError("no result")
    report(stream, result, values)
    return result, values
"#,
            "async_terminal_elif_linear_prefix_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_guarded_recovery_finally_joins() {
    for (source, filename) in [
        (
            r"def collect(resource, work, recover, report, enabled, retry):
    try:
        try:
            value = work(resource)
            if enabled:
                report(value)
        except Exception:
            if not retry:
                raise
            fallback = recover(resource)
            try:
                work(fallback)
                report(fallback)
            finally:
                fallback.close(missing_ok=True)
    finally:
        resource.close(missing_ok=True)
    report(resource)
    report(enabled, retry)
    return resource
",
            "guarded_recovery_linear_finalizer_target.py",
        ),
        (
            r#"def collect(resource, work, report, enabled, option, retry):
    try:
        try:
            value = work(resource, option=option if enabled else None)
            if value.failed:
                raise ValueError("failed")
            report(value)
            errors = []
            if enabled and option:
                errors = [{"error": "incomplete"}]
        except Exception as error:
            if not retry:
                raise
            errors = [{"error": str(error)}]
    finally:
        resource.close(missing_ok=True)
    report(resource, errors)
    if errors:
        report(errors)
    return errors
"#,
            "guarded_recovery_compound_join_target.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_conditional_finalizer_call_suffixes() {
    for (source, filename) in [
        (
            r#"def finish(work, worker, resource, connect, endpoint, report):
    value = None
    try:
        value = work(endpoint)
    finally:
        if worker.is_alive():
            try:
                with connect(endpoint, timeout=1):
                    pass
            except OSError:
                pass
        worker.join(timeout=5)
        resource.close()
    if worker.is_alive():
        raise RuntimeError("worker active")
    if value is None:
        raise RuntimeError("missing result")
    report(value, endpoint)
    return value
"#,
            "finally_guarded_manager_call_suffix_target.py",
        ),
        (
            r#"def finish(work, worker, resource, connect, endpoint, report):
    value = None
    try:
        value = work(endpoint)
    finally:
        worker.join(timeout=5)
        resource.close()
    if worker.is_alive():
        raise RuntimeError("worker active")
    if value is None:
        raise RuntimeError("missing result")
    report(value, endpoint)
    return value
"#,
            "finally_guarded_manager_call_suffix_linear_control.py",
        ),
        (
            r#"def finish(work, worker, pending, token, report, full_error):
    problem = None
    try:
        with work() as jobs:
            for job in jobs:
                try:
                    job.run()
                except BaseException as error:
                    problem = error
                    job.stop()
                    break
            if problem is not None:
                for job in jobs:
                    job.cancel()
    finally:
        if worker.is_alive():
            while True:
                try:
                    pending.put(token, timeout=0.5)
                    break
                except full_error:
                    if not worker.is_alive():
                        break
        worker.join()
    if problem is not None:
        raise RuntimeError("work failed") from problem
    report(token)
    return token
"#,
            "finally_conditional_retry_call_suffix_target.py",
        ),
        (
            r#"def finish(work, worker, pending, token, report, full_error):
    problem = None
    try:
        with work() as jobs:
            for job in jobs:
                try:
                    job.run()
                except BaseException as error:
                    problem = error
                    job.stop()
                    break
            if problem is not None:
                for job in jobs:
                    job.cancel()
    finally:
        worker.join()
    if problem is not None:
        raise RuntimeError("work failed") from problem
    report(token)
    return token
"#,
            "finally_conditional_retry_call_suffix_linear_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_managed_while_reraise_finally() {
    for (source, filename) in [
        (
            r"async def collect(outer, manager, resource, pending, wait, report):
    async with outer:
        try:
            with manager:
                try:
                    while pending:
                        completed = await wait(pending)
                        for task in completed:
                            report(task)
                except Exception:
                    for task in pending:
                        task.cancel()
                    await wait(pending)
                    raise
        finally:
            if resource is not None:
                resource.close()
    report(resource)
",
            "managed_while_reraise_conditional_finally_target.py",
        ),
        (
            r"async def collect(outer, manager, resource, pending, wait, report):
    async with outer:
        try:
            with manager:
                try:
                    while pending:
                        completed = await wait(pending)
                        for task in completed:
                            report(task)
                except Exception:
                    for task in pending:
                        task.cancel()
                    await wait(pending)
                    report(pending)
        finally:
            if resource is not None:
                resource.close()
    report(resource)
",
            "managed_while_recover_conditional_finally_control.py",
        ),
        (
            r"async def collect(manager, resource, pending, wait, report, value):
    def retain_resource():
        return resource

    try:
        source = pending
        with manager:
            try:
                for task in source:
                    await wait(task)
                while pending:
                    await wait(pending)
            except Exception:
                for task in pending:
                    task.cancel()
                if pending:
                    await wait(pending)
                raise
    finally:
        await resource.close()
    report(value)
    return value
",
            "managed_while_reraise_awaited_finally_target.py",
        ),
        (
            r"async def collect(manager, resource, pending, wait, report, value):
    def retain_resource():
        return resource

    try:
        source = pending
        with manager:
            try:
                for task in source:
                    await wait(task)
                while pending:
                    await wait(pending)
            except Exception:
                for task in pending:
                    task.cancel()
                if pending:
                    await wait(pending)
                report(pending)
    finally:
        await resource.close()
    report(value)
    return value
",
            "managed_while_recover_awaited_finally_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_nested_named_recovery_finally() {
    for (source, filename) in [
        (
            r"def publish(resource, work, acquire, restore, report, enabled):
    try:
        report(resource)
        try:
            work(resource, mode=enabled or resource)
            completed = True
            report(completed)
        except Exception as error:
            code = report(error)
            if code != 0 and not enabled:
                raise
            report(error)
            saved = restore(resource)
            if saved:
                raise
            try:
                restore(resource)
                resource = None
                retry = acquire()
                try:
                    work(retry)
                    completed = True
                    report(completed)
                finally:
                    retry.close()
            finally:
                restore(saved)
    finally:
        if resource:
            resource.close()
",
            "nested_named_guarded_restoration_finally_target.py",
        ),
        (
            r"def run(worker, initial, load, report):
    try:
        if worker.offline:
            with load() as items:
                for item in items:
                    report(item)
            return
        for item in initial:
            report(item)
        try:
            while not worker.stop() and not worker.closed():
                for item in worker.items():
                    report(item)
        except Exception as error:
            report(error)
    finally:
        worker.stopped.set()
",
            "nested_named_multiedge_while_finally_target.py",
        ),
        (
            r"def run(worker, initial, load, report):
    try:
        if worker.offline:
            with load() as items:
                for item in items:
                    report(item)
            return
        for item in initial:
            report(item)
        try:
            while not worker.stop():
                for item in worker.items():
                    report(item)
        except Exception as error:
            report(error)
    finally:
        worker.stopped.set()
",
            "nested_named_singleedge_while_finally_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_named_cleanup_store_suffix() {
    for (source, filename) in [
        (
            r#"def build(worker, report, kind):
    source = "direct"
    try:
        result = worker.build(kind)
    except Exception as error:
        report(error)
        source = "fallback"
        try:
            spec = worker.retry(kind)
        except Exception as inner:
            report(inner)
            saved = worker.method
            try:
                worker.method = lambda value: -1
                spec = worker.retry(kind)
            finally:
                worker.method = saved
        result = {"spec": spec, "source": source, "tokens": list(worker.tokens())}
    return result, source, kind
"#,
            "named_nested_restoration_store_suffix_target.py",
        ),
        (
            r"def lease(resource, acquire, manager, report):
    payload = resource.payload
    try:
        handle = acquire(resource)
    except OSError as error:
        existing = resource.read()
        if existing.active:
            if existing.owner:
                try:
                    yield existing
                finally:
                    try:
                        current = resource.read()
                        if current.active and current.owner:
                            if existing.restore:
                                report(existing)
                            else:
                                with manager():
                                    resource.remove()
                    finally:
                        report(resource)
                return
            reason = existing.reason
            raise RuntimeError(reason) from error
        with manager():
            resource.remove()
        handle = acquire(resource)
    with resource.open(handle) as output:
        output.write(payload)
    report(resource)
    try:
        yield handle
    finally:
        try:
            current = resource.read()
            if current.active and current.owner:
                with manager():
                    resource.remove()
        finally:
            report(resource)
",
            "named_manager_store_suffix_target.py",
        ),
        (
            r#"def build(worker, report, kind):
    source = "direct"
    try:
        result = worker.build(kind)
    except Exception as error:
        report(error)
        source = "fallback"
        try:
            spec = worker.retry(kind)
        except Exception as inner:
            report(inner)
            saved = worker.method
            worker.method = lambda value: -1
            spec = worker.retry(kind)
            worker.method = saved
        result = {"spec": spec, "source": source, "tokens": list(worker.tokens())}
    return result, source, kind
"#,
            "named_nested_linear_store_suffix_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_through_live_recovery_entries() {
    for (source, filename) in [
        (
            r#"def run(recorder, worker, sample, index, record_raw):
    with recorder.open():
        if record_raw:
            try:
                recorder.record(sample)
            except Exception as error:
                if worker.ignore:
                    recorder.report(error)
                else:
                    raise
        seed = worker.seed(index)
        try:
            result = worker.run(sample, seed)
        except Exception as error:
            partial = getattr(error, "partial", None)
            if partial is not None:
                try:
                    recorder.record(partial)
                except Exception:
                    recorder.report(sample)
            recorder.report(error)
            if not worker.accepts(error):
                recorder.report(sample)
                raise
            result = None
            recorder.report(error)
        recorder.completed(index)
        return index, result
"#,
            "managed_guarded_linear_recovery_suffix_target.py",
        ),
        (
            r#"def validate(load, parse, attest, changed, valid, fields):
    errors = []
    try:
        data = load()
    except OSError as error:
        errors.append(str(error))
    else:
        if changed(data):
            errors.append("changed")
        try:
            value = parse(data)
        except ValueError as error:
            errors.append(str(error))
        else:
            if not valid(value):
                errors.append("invalid")
            else:
                for field in fields:
                    if changed(field):
                        errors.append(field)
                try:
                    result = attest(value)
                except RuntimeError as error:
                    errors.append(str(error))
                else:
                    if changed(result):
                        errors.append("attested")
    if errors:
        raise RuntimeError("; ".join(errors))
"#,
            "nested_try_else_populated_call_entry_target.py",
        ),
        (
            r#"def run(recorder, worker, sample, index, record_raw):
    with recorder.open():
        if record_raw:
            try:
                recorder.record(sample)
            except Exception as error:
                if worker.ignore:
                    recorder.report(error)
                else:
                    raise
        seed = worker.seed(index)
        try:
            result = worker.run(sample, seed)
        except Exception as error:
            partial = getattr(error, "partial", None)
            if partial is not None:
                try:
                    recorder.record(partial)
                except Exception:
                    raise
            recorder.report(error)
            if not worker.accepts(error):
                recorder.report(sample)
                raise
            result = None
            recorder.report(error)
        recorder.completed(index)
        return index, result
"#,
            "managed_guarded_raising_recovery_suffix_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

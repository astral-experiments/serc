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

#[test]
fn matches_cpython_borrows_after_recovery_value_matches() {
    for (source, filename) in [
        (
            r#"def edit(value, read, parse, choose, report):
    while True:
        text = read(value)
        if text is None:
            return
        try:
            result = parse(text)
        except ValueError as error:
            report(error)
            action = choose()
            match action:
                case "again":
                    continue
                case "reset":
                    value = text
                    continue
                case "stop":
                    report(error)
                    return
                case _:
                    raise ValueError(action) from None
        formatted = str(result)
        report(formatted, value)
        if choose():
            return formatted
"#,
            "named_terminal_value_match_recovery_target.py",
        ),
        (
            r#"def edit(value, read, parse, choose, report):
    while True:
        text = read(value)
        if text is None:
            return
        try:
            result = parse(text)
        except ValueError as error:
            report(error)
            action = choose()
            if action == "again":
                continue
            elif action == "reset":
                value = text
                continue
            elif action == "stop":
                report(error)
                return
            else:
                raise ValueError(action) from None
        formatted = str(result)
        report(formatted, value)
        if choose():
            return formatted
"#,
            "named_terminal_if_recovery_control.py",
        ),
        (
            r#"def collect(messages, decode, report):
    seen = []
    for message in messages:
        first = False
        second = False
        try:
            values = decode(message)
            for key, items in values.items():
                size = len(items) if isinstance(items, list) else 1
                report(key, size)
                if key.startswith("alpha"):
                    first = True
                elif key.startswith("beta"):
                    second = True
                elif key == "other":
                    first = True
        except Exception:
            for line in message.splitlines():
                key, *values = line.split("|")
                if values:
                    report(key, len(values))
                match key:
                    case "slow":
                        first = True
                    case "fast":
                        second = True
        report(message, first, second)
        seen.append((first, second))
    return seen
"#,
            "iterator_value_match_recovery_target.py",
        ),
        (
            r#"def collect(messages, decode, report):
    seen = []
    for message in messages:
        first = False
        second = False
        try:
            values = decode(message)
            for key, items in values.items():
                size = len(items) if isinstance(items, list) else 1
                report(key, size)
                if key.startswith("alpha"):
                    first = True
                elif key.startswith("beta"):
                    second = True
                elif key == "other":
                    first = True
        except Exception:
            for line in message.splitlines():
                key, *values = line.split("|")
                if values:
                    report(key, len(values))
                if key == "slow":
                    first = True
                if key == "fast":
                    second = True
        report(message, first, second)
        seen.append((first, second))
    return seen
"#,
            "iterator_independent_if_recovery_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_iterator_reduction_and_recovery() {
    for (source, filename) in [
        (
            r#"async def collect(stream, check, report):
    values = []
    try:
        async for item in stream:
            if item < 0:
                raise ValueError(item)
            elif item:
                values.append(item)
                matched = any(value > 0 and check(value) for value in values)
                if matched:
                    break
        report(values)
        report(len(values))
        return values
    except ValueError:
        report("recovered")
    return []
"#,
            "async_reduction_assignment_terminal_break_target.py",
        ),
        (
            r#"async def collect(stream, check, report):
    values = []
    try:
        async for item in stream:
            if item < 0:
                raise ValueError(item)
            elif item:
                values.append(item)
                matched = check(values)
                if matched:
                    break
        report(values)
        report(len(values))
        return values
    except ValueError:
        report("recovered")
    return []
"#,
            "async_linear_assignment_terminal_break_control.py",
        ),
        (
            r#"def collect(source, accept, report):
    found = False
    values = []
    try:
        if source.items is not None:
            for item in source.items:
                content = getattr(item, "content", None)
                if accept(content):
                    found = True
                    try:
                        key = getattr(content, "key", None)
                        value = getattr(content, "value", None)
                        if isinstance(key, str) and isinstance(value, str) and value:
                            values.append((key, value))
                    except Exception:
                        pass
                    break
    except Exception:
        found = False
    report(found, values)
    if found and values:
        return values
    return []
"#,
            "guarded_iterator_pass_recovery_break_target.py",
        ),
        (
            r#"def collect(source, accept, report):
    found = False
    values = []
    try:
        if source.items is not None:
            for item in source.items:
                content = getattr(item, "content", None)
                if accept(content):
                    found = True
                    report(content)
                    break
    except Exception:
        found = False
    report(found, values)
    if found and values:
        return values
    return []
"#,
            "guarded_iterator_linear_break_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_guarded_nested_recovery() {
    for (source, filename) in [
        (
            r#"def recover(text, parse, retry, repair, needed, report):
    try:
        parsed = parse(text)
        recovery = {"attempted": False}
    except ValueError as error:
        attempts = []
        parsed = None
        if needed(text):
            try:
                parsed, result = retry(text, error)
                attempts.append(result)
                recovery = {"attempts": attempts}
            except Exception as retry_error:
                attempts.append(str(retry_error))
        if parsed is None:
            try:
                parsed, result = repair(text, error)
                attempts.append(result)
                recovery = {"attempts": attempts}
            except Exception as repair_error:
                attempts.append(str(repair_error))
                parsed, result = retry(text, repair_error)
                attempts.append(result)
                recovery = {"attempts": attempts}
    parsed["raw"] = text
    report(parsed, recovery)
    return parsed
"#,
            "named_two_guard_nested_recovery_target.py",
        ),
        (
            r#"def recover(text, parse, retry, repair, needed, report):
    try:
        parsed = parse(text)
        recovery = {"attempted": False}
    except ValueError as error:
        attempts = []
        parsed = None
        if needed(text):
            try:
                parsed, result = retry(text, error)
                attempts.append(result)
                recovery = {"attempts": attempts}
            except Exception as retry_error:
                recovery = None
        if parsed is None:
            try:
                parsed, result = repair(text, error)
                attempts.append(result)
                recovery = {"attempts": attempts}
            except Exception as repair_error:
                recovery = None
    parsed["raw"] = text
    report(parsed, recovery)
    return parsed
"#,
            "named_two_guard_store_recovery_control.py",
        ),
        (
            r#"def recover(work, retry, report):
    process = None
    result = ""
    try:
        process = work()
        result = process.read()
        report(result)
    except TimeoutError as error:
        result = str(error)
        if process is not None:
            token = None
            try:
                token = process.identity()
                report(token)
            except LookupError:
                report(process)
            try:
                result = process.read()
                report(result)
                result = process.result if process.result is not None else result
            except TimeoutError as second_error:
                report(second_error)
                result = retry(process)
                result = result if result is not None else ""
    except OSError as error:
        result = str(error)
    report(result, process)
    return result
"#,
            "named_none_guard_nested_recovery_target.py",
        ),
        (
            r#"def recover(work, retry, report):
    process = None
    result = ""
    try:
        process = work()
        result = process.read()
        report(result)
    except TimeoutError as error:
        result = str(error)
        if process is not None:
            token = None
            try:
                token = process.identity()
                report(token)
            except LookupError:
                token = None
            try:
                result = process.read()
                report(result)
                result = process.result if process.result is not None else result
            except TimeoutError as second_error:
                result = ""
    except OSError as error:
        result = str(error)
    report(result, process)
    return result
"#,
            "named_none_guard_store_recovery_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_borrows_after_async_guarded_recovery_else() {
    for (source, filename) in [
        (
            r#"async def probe(manager, read, restore, report, opened, changed, restored):
    try:
        async with manager():
            opened = await read()
            if opened:
                changed = await restore()
                if not changed:
                    raise RuntimeError("change failed")
            else:
                restored = await read()
                if not restored:
                    raise RuntimeError("restore failed")
    except Exception as error:
        if changed and not restored:
            try:
                async with manager():
                    restored = await restore()
                report(restored)
            except Exception as recovery_error:
                report(recovery_error)
        status = {"ok": False, "error": str(error), "restored": restored}
    else:
        status = {"ok": True, "restored": restored}
    status["opened"] = opened
    report(status)
    return status
"#,
            "async_guarded_recovery_populated_else_target.py",
        ),
        (
            r#"async def probe(manager, read, restore, report, opened, changed, restored):
    try:
        async with manager():
            opened = await read()
            if opened:
                changed = await restore()
                if not changed:
                    raise RuntimeError("change failed")
            else:
                restored = await read()
                if not restored:
                    raise RuntimeError("restore failed")
    except Exception as error:
        if changed and not restored:
            try:
                async with manager():
                    restored = await restore()
                report(restored)
            except Exception as recovery_error:
                restored = False
        status = {"ok": False, "error": str(error), "restored": restored}
    else:
        status = {"ok": True, "restored": restored}
    status["opened"] = opened
    report(status)
    return status
"#,
            "async_guarded_store_recovery_populated_else_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

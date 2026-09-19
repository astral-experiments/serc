use crate::common;

#[test]
fn matches_cpython_marshal_for_retained_setup_null_append_and_owned_controls() {
    let cases = [
        (
            "two_managers_local_condition.py",
            r"async def f(flag, first, second, timeout):
    if flag:
        prepare()
    with first:
        with second:
            try:
                if timeout is None:
                    await run()
                else:
                    await other()
            except TimeoutError:
                raise
            finally:
                await close()
",
        ),
        (
            "one_manager_attribute_condition.py",
            r"async def f(flag, first, obj):
    if flag:
        prepare()
    with first:
        try:
            if obj.timeout is None:
                await run()
            else:
                await other()
        except TimeoutError:
            raise
        finally:
            await close()
",
        ),
        (
            "one_manager_local_condition.py",
            r"async def f(flag, first, timeout):
    if flag:
        prepare()
    with first:
        try:
            if timeout is None:
                await run()
            else:
                await other()
        except TimeoutError:
            raise
        finally:
            await close()
",
        ),
        (
            "two_managers_attribute_condition.py",
            r"async def f(flag, first, second, obj):
    if flag:
        prepare()
    with first:
        with second:
            try:
                if obj.timeout is None:
                    await run()
                else:
                    await other()
            except TimeoutError:
                raise
            finally:
                await close()
",
        ),
        (
            "intervening_branch_control.py",
            r"async def f(flag, first, second, timeout):
    with first:
        with second:
            if flag:
                prepare()
            try:
                if timeout is None:
                    await run()
            except TimeoutError:
                raise
            finally:
                await close()
",
        ),
        (
            "copied_finally_control.py",
            r"async def f(flag, first, second, timeout):
    try:
        if flag:
            return
    finally:
        with first:
            with second:
                try:
                    if timeout is None:
                        await run()
                except TimeoutError:
                    raise
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
fn matches_cpython_marshal_for_cold_awaited_manager_pop_block() {
    let cases = [
        (
            "cold_single_manager_awaited_finally.py",
            r"async def f(flag, manager, work, close):
    try:
        if flag:
            return
    finally:
        with manager:
            try:
                work()
            except ValueError:
                raise
            finally:
                await close()
",
        ),
        (
            "warm_manager_awaited_finally.py",
            r"async def f(manager, work, close):
    with manager:
        try:
            work()
        except ValueError:
            raise
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
fn marker2177_probe_full_function() {
    common::assert_matches_cpython_marshal(
        r#"def crasher(timeout: float | None, name: str | None = None):
    if timeout is None:
        yield
        return
    create_time = psutil.Process(os.getpid()).create_time()
    log_file = os.path.join(os.environ["HOME"], ".crasher_logs", f"{os.getpid()}.log")
    kill_eventually = True

    os.makedirs(os.path.dirname(log_file), exist_ok=True)
    with open(log_file, "w") as f:
        process = subprocess.Popen(
            [
                "python",
                "-m",
                __loader__.name,
                str(time.time() + timeout),
                str(os.getpid()),
                str(create_time),
            ],
            stdin=subprocess.DEVNULL,
            stdout=f,
            stderr=subprocess.STDOUT,
        )
        try:
            yield
        except KeyboardInterrupt:
            # don't kill in case we hang still
            label = f": {name}" if name is not None else ""
            logger.info(f"Likely interrupted by crasher{label} (timeout: {timeout} seconds)")
            kill_eventually = False
            raise
        finally:
            if kill_eventually:
                process.kill()
                process.wait()
"#,
        "marker2177_full_function.py",
    );
}

#[test]
fn marker2177_probe_minimal() {
    common::assert_matches_cpython_marshal(
        r"def f(manager, flag):
    with manager:
        try:
            yield
        except ValueError:
            flag = False
            raise
        finally:
            if flag:
                close()
                wait()
",
        "marker2177_minimal.py",
    );
}

#[test]
fn marker2177_probe_prefix() {
    common::assert_matches_cpython_marshal(
        r"def f(manager, flag):
    if flag is None:
        yield
        return
    flag = True
    with manager:
        prepare()
        try:
            yield
        except ValueError:
            report()
            flag = False
            raise
        finally:
            if flag:
                close()
                wait()
",
        "marker2177_prefix.py",
    );
}

#[test]
fn matches_cpython_uncompacted_conditional_arrays() {
    common::assert_matches_cpython_marshal(
        r#"def f(items):
    try:
        results = []
        for item in items:
            if item:
                try:
                    results.append({"value": item.value if item.value else ""})
                except Exception:
                    continue
    except Exception:
        raise
"#,
        "4861_07_no_return.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(state):
    while True:
        if state.cancel:
            break
    try:
        if state.future.done():
            state.future = work()
        yield state.future
    finally:
        state.count -= 1
",
        "6469_04_break_loop.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(running, queue):
    try:
        empty = True
        while running.value or not empty:
            try:
                if not running.value:
                    queue.get(timeout=2)
                else:
                    queue.get_nowait()
                empty = False
            except Empty:
                empty = True
                await sleep(0)
    except KeyboardInterrupt:
        pass
",
        "4890_04_no_value_store.py",
    );
    common::assert_matches_cpython_marshal(
        r#"try:
    with manager():
        data = read(path)
    if not data:
        warn()
    else:
        size = None
        try:
            if path.startswith("prefix:") and helper is not None:
                size = first()
            else:
                size = second()
        except Exception:
            pass
        report(size)
except Exception:
    report(None)
"#,
        "6697_08_no_outer_guard.py",
    );
}

#[test]
fn matches_cpython_comprehension_restore_then_retained_setup() {
    common::assert_matches_cpython_marshal(
        r"async def f(values):
    selected = sorted({item for item in values if item})
    try:
        result = await consume(selected) if selected else {}
    except Exception:
        result = {}
    return result
",
        "async_setcomp_then_try_expr.py",
    );
    common::assert_matches_cpython_marshal(
        r#"async def f(value):
    disabled = value in {"a", "b", "c"}
    staged = False
    try:
        if not disabled:
            staged = await consume(value)
    except Exception:
        staged = False
    return staged
"#,
        "async_contains_then_try_if.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(value):
    ready = False
    try:
        ready = await consume(value)
    finally:
        try:
            if not ready:
                await cleanup(value)
        finally:
            await finish(value)
",
        "async_nested_finally_try_if.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(value):
    ready = value
    try:
        if ready:
            return await consume(value)
    except Exception:
        return None
    return value
",
        "async_plain_assignment_try_if.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(value):
    ready = value
    try:
        if ready:
            return consume(value)
    except Exception:
        return None
    return value
",
        "sync_plain_assignment_try_if.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(value):
    def capture():
        return value
    ready = value
    try:
        result = consume(ready) if ready else []
    except Exception:
        result = []
    return capture, result
",
        "sync_cell_assignment_try_expr.py",
    );
}

#[test]
fn matches_cpython_comprehension_restore_without_retained_setup() {
    for (filename, source) in [
        (
            "with_loop_any_listcomp_condition.py",
            r"def f(manager, groups):
    with manager:
        for values in groups:
            if any([value for value in values]):
                consume(values)
",
        ),
        (
            "with_loop_not_all_listcomp_condition.py",
            r"def f(manager, groups):
    with manager:
        for values in groups:
            if not all([value for value in values]):
                continue
            consume(values)
",
        ),
        (
            "with_loop_any_plain_argument_control.py",
            r"def f(manager, groups):
    with manager:
        for values in groups:
            if any(values):
                consume(values)
",
        ),
        (
            "with_loop_not_all_plain_argument_control.py",
            r"def f(manager, groups):
    with manager:
        for values in groups:
            if not all(values):
                continue
            consume(values)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_normal_finally_owner_after_comprehension_restore() {
    for (filename, source) in [
        (
            "normal_finally_dictcomp_restore_prefix.py",
            r"def f(resource, pairs, sink):
    try:
        mapped = {str(key): int(value) for key, value in pairs}
        result = third(second(first(mapped)))
    finally:
        if resource is not None:
            resource.close()
    sink(result)
",
        ),
        (
            "normal_finally_dictcomp_local_call_control.py",
            r"def f(resource, pairs, sink, first, second, third):
    try:
        mapped = {str(key): int(value) for key, value in pairs}
        result = third(second(first(mapped)))
    finally:
        if resource is not None:
            resource.close()
    sink(result)
",
        ),
        (
            "normal_finally_global_call_folds.py",
            r"def f(resource, value):
    try:
        result = third(second(first(value)))
    finally:
        if resource is not None:
            resource.close()
    consume(result)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_finally_return_owner_before_original_nop() {
    for (filename, source) in [
        (
            "finally_return_multiline_global_print_prefix.py",
            r#"def target(options, path, value):
    try:
        if options.skip:
            return 0
        result = run(
            value,
            mode=options.mode,
            enabled=options.enabled,
        )
        print(f"output={result.output}")
        print(f"details={result.details}")
        print(f"report={result.report}")
        print(f"url={result.url}")
        print(f"manifest={result.manifest}")
        print(f"count={result.count}")
        print(f"average={result.average}")
        return 0
    finally:
        if not options.keep:
            try:
                path.unlink(missing_ok=True)
                path.parent.rmdir()
            except OSError:
                pass
"#,
        ),
        (
            "finally_return_global_print_prefix.py",
            r#"def target(options, path, value):
    try:
        if options.skip:
            return 0
        result = run(value, mode=options.mode, enabled=options.enabled)
        print(f"output={result.output}")
        print(f"details={result.details}")
        print(f"report={result.report}")
        print(f"url={result.url}")
        print(f"manifest={result.manifest}")
        print(f"count={result.count}")
        print(f"average={result.average}")
        return 0
    finally:
        if not options.keep:
            try:
                path.unlink(missing_ok=True)
                path.parent.rmdir()
            except OSError:
                pass
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_finally_owner_after_return_pop() {
    for (filename, source) in [
        (
            "finally_nested_return_multiline_prefix.py",
            r#"def target(local_path, root, metadata, destination, stats, count):
    try:
        if local_path is None:
            return []
        frames = list(
            extract(
                local_path,
                root,
                count,
                stats=stats,
                enabled=True,
            )
        )
        started = clock.time()
        uploaded = upload(
            frames,
            destination,
        )
        elapsed = clock.time() - started
        print(
            f"id={metadata.get('id')} "
            f"bytes={metadata.get('size')} "
            f"frames={len(frames)} "
            f"elapsed={elapsed:.2f}"
        )
        return uploaded
    finally:
        try:
            cleanup.remove_tree(root, ignore_errors=True)
        finally:
            if local_path:
                try:
                    cleanup.remove(local_path)
                except Exception:
                    pass
"#,
        ),
        (
            "finally_nested_return_global_prefix.py",
            r#"def target(local_path, root, metadata, destination, stats, count):
    try:
        if local_path is None:
            return []
        frames = list(extract(local_path, root, count, stats=stats, enabled=True))
        started = clock.time()
        uploaded = upload(frames, destination)
        elapsed = clock.time() - started
        print(
            f"id={metadata.get('id')} "
            f"bytes={metadata.get('size')} "
            f"frames={len(frames)} "
            f"elapsed={elapsed:.2f}"
        )
        return uploaded
    finally:
        try:
            cleanup.remove_tree(root, ignore_errors=True)
        finally:
            if local_path:
                try:
                    cleanup.remove(local_path)
                except Exception:
                    pass
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_normal_finally_owner_after_saturated_literal_fold() {
    for (filename, source) in [
        (
            "normal_finally_guarded_high_local_unpack.py",
            r#"def target(
    p00, p01, p02, p03, p04, p05, p06,
    p07, p08, p09, p10, p11, p12, p13,
    p14, p15, p16, p17, p18, p19, p20,
    p21, p22, p23, p24, p25, p26,
):
    try:
        if p26 is None:
            raise ValueError("measurement unavailable")
        q0, q1, q2, q3, q4, q5 = p15.quantiles(
            "value",
            [0.0001, 0.001, 0.01, 0.10, 0.50, 0.90],
            float(p03),
        )
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
    finally:
        if p15 is not None:
            p15.close()
        if p21 is not None:
            p21.close()
        if p20 is not None:
            p20.close()
        if p18 is not None:
            p18.close()
        if p19 is not None:
            p19.close()
        if p17 is not None:
            p17.close()
        if p16 is not None:
            p16.close()
        p14.close()
        p13.close()
        p12.close()
        p11.close()
        p10.close()
        p09.close()
        p08.close()
        p07.close()
"#,
        ),
        (
            "normal_finally_guarded_local_calls_control.py",
            r#"def target(
    p00, p01, p02, p03, p04, p05, p06,
    p07, p08, p09, p10, p11, p12, p13,
    p14, p15, p16, p17, p18, p19, p20,
    p21, p22, p23, p24, p25, p26, float,
    emit, convert,
):
    try:
        if p26 is None:
            raise ValueError("measurement unavailable")
        q0, q1, q2, q3, q4, q5 = p15.quantiles(
            "value",
            [0.0001, 0.001, 0.01, 0.10, 0.50, 0.90],
            float(p03),
        )
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
    finally:
        if p15 is not None:
            p15.close()
        if p21 is not None:
            p21.close()
        if p20 is not None:
            p20.close()
        if p18 is not None:
            p18.close()
        if p19 is not None:
            p19.close()
        if p17 is not None:
            p17.close()
        if p16 is not None:
            p16.close()
        p14.close()
        p13.close()
        p12.close()
        p11.close()
        p10.close()
        p09.close()
        p08.close()
        p07.close()
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_condition_after_original_comprehension_restore_span() {
    for (filename, source) in [
        (
            "async_cell_setcomp_then_try_expr.py",
            r"async def f(values):
    selected = sorted({(lambda: item) for item in values if item})
    try:
        result = await consume(selected) if selected else {}
    except Exception:
        result = {}
    return result
",
        ),
        (
            "async_generator_setcomp_then_try_expr.py",
            r"async def f(values):
    selected = sorted({item async for item in values if item})
    try:
        result = await consume(selected) if selected else {}
    except Exception:
        result = {}
    return result
",
        ),
        (
            "async_two_restore_setcomp_then_try_expr.py",
            r"async def f(values):
    selected = sorted({(left, right) for left in values for right in values if right})
    try:
        result = await consume(selected) if selected else {}
    except Exception:
        result = {}
    return result
",
        ),
        (
            "async_setcomp_then_try_expr.py",
            r"async def f(values):
    selected = sorted({item for item in values if item})
    try:
        result = await consume(selected) if selected else {}
    except Exception:
        result = {}
    return result
",
        ),
        (
            "async_setcomp_without_call_then_try_expr.py",
            r"async def f(values):
    selected = {item for item in values if item}
    try:
        result = await consume(selected) if selected else {}
    except Exception:
        result = {}
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_value_returns_after_retained_setup() {
    for (filename, source) in [
        (
            "five_managers_retained_setup_intervening_call_control.py",
            r"def f(a, b, c, d, e, extra, flag, value, fallback, audit, finish):
    if flag:
        audit()
    with a, b, c, d, e:
        try:
            audit()
            result = value or fallback()
            return result
        finally:
            finish()
",
        ),
        (
            "five_managers_retained_setup_value_and_null_target.py",
            r"def f(a, b, c, d, e, extra, flag, value, fallback, audit, finish):
    if flag:
        audit()
    with a, b, c, d, e:
        try:
            result = value and fallback()
            return result
        finally:
            finish()
",
        ),
        (
            "five_managers_retained_setup_value_or_null_target.py",
            r"def f(a, b, c, d, e, extra, flag, value, fallback, audit, finish):
    if flag:
        audit()
    with a, b, c, d, e:
        try:
            result = value or fallback()
            return result
        finally:
            finish()
",
        ),
        (
            "four_managers_retained_setup_value_owner_control.py",
            r"def f(a, b, c, d, e, extra, flag, value, fallback, audit, finish):
    if flag:
        audit()
    with a, b, c, d:
        try:
            result = value or fallback()
            return result
        finally:
            finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_uncompacted_conditional_copy_retargets() {
    for (filename, source) in [
        (
            "uncompacted_copied_exit_nested_try_owner_target.py",
            r#"async def record_history(read, history, store, enabled, limit):
    while True:
        answer = read()
        if answer is None:
            break
        try:
            if answer:
                compact = answer if len(answer) <= limit else answer[:limit]
                history.append(f"Q: {answer}")
                history.append(f"A: {compact}")
                try:
                    if enabled and store is not None:
                        store.append_string(answer)
                except Exception:
                    pass
        except Exception:
            pass
"#,
        ),
        (
            "uncompacted_copied_exit_nested_try_sync_control.py",
            r#"def record_history(read, history, store, enabled, limit):
    while True:
        answer = read()
        if answer is None:
            break
        try:
            if answer:
                compact = answer if len(answer) <= limit else answer[:limit]
                history.append(f"Q: {answer}")
                history.append(f"A: {compact}")
                try:
                    if enabled and store is not None:
                        store.append_string(answer)
                except Exception:
                    pass
        except Exception:
            pass
"#,
        ),
        (
            "uncompacted_copied_exit_live_join_control.py",
            r#"async def record_history(read, history, store, enabled, limit):
    while True:
        answer = read()
        if answer is None:
            break
        try:
            if answer:
                compact = answer if len(answer) <= limit else answer[:limit]
                history.append(f"Q: {answer}")
                history.append(f"A: {compact}")
                try:
                    if enabled and store is not None:
                        store.append_string(answer)
                except Exception:
                    pass
                history.append("done")
        except Exception:
            pass
"#,
        ),
        (
            "uncompacted_copied_exit_compacted_prefix_control.py",
            r#"async def record_history(read, history, store, enabled, limit):
    while True:
        answer = read()
        if answer is None:
            break
        try:
            if answer:
                compact = answer if len(answer) <= limit else answer[:limit]
                question, response = answer, compact
                history.append(f"Q: {question}")
                history.append(f"A: {response}")
                try:
                    if enabled and store is not None:
                        store.append_string(answer)
                except Exception:
                    pass
        except Exception:
            pass
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_retained_setup_after_local_fusion() {
    for (filename, source) in [
        (
            "iterator_try_ifexp_post_fusion_owner.py",
            r"def target(rows, options):
    for index, row in rows:
        publish(path(options), index, row)
        report(stamp())
        started = clock.time()
        try:
            result = prepare(row, options) if options.prepare else run(row, options)
            consume(result, started)
        except Exception as error:
            recover(error)
",
        ),
        (
            "iterator_try_ifexp_no_local_fusion_control.py",
            r"def target(rows, options):
    for row in rows:
        publish(path(options), row)
        report(stamp())
        started = clock.time()
        try:
            result = prepare(row, options) if options.prepare else run(row, options)
            consume(result, started)
        except Exception as error:
            recover(error)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_retained_setup_array_boundaries() {
    for (filename, source) in [
        (
            "nested_retained_setup_global_calls_target.py",
            r"def f():
    try:
        first()
        second()
        third()
        fourth()
        fifth()
        value = SOURCE
        try:
            if ENABLED:
                consume(value)
        finally:
            inner_cleanup()
    finally:
        outer_cleanup()
",
        ),
        (
            "nested_retained_setup_new_array_control.py",
            r"def f(branch):
    try:
        first()
        second()
        third()
        fourth()
        fifth()
        value = SOURCE
        if branch:
            value = OTHER
        try:
            if ENABLED:
                consume(value)
        finally:
            inner_cleanup()
    finally:
        outer_cleanup()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_retained_setup_after_unprotected_record_work() {
    for (filename, source) in [
        (
            "sequential_named_try_attribute_ifexp_target.py",
            r"def run(groups, allowed):
    output = {}
    records = {}
    for group_index, group in enumerate(groups):
        for item in group:
            if item.kind in allowed:
                if group_index not in output:
                    output[group_index] = []
                try:
                    first = transform(item.value) if item.value else None
                except AttributeError as error:
                    report(item.value)
                    report(error)
                    first = None
                output[group_index].append(Entry(value=item.value, filtered=first))
                if item.key not in records:
                    records[item.key] = Record(
                        before=None, after=None, weight=None,
                        key=item.key, value=None, latest=None,
                    )
                    records[item.key].before = Content(
                        value=load_before(item.value), key=item.key,
                    )
                records[item.key].latest = parse(item.value)
                records[item.key].after = Content(
                    value=load_after(item.value), key=item.key,
                )
                records[item.key].weight = measure(item.value)
                merged = combine(
                    records[item.key].before.value,
                    records[item.key].after.value,
                    item.key,
                )
                try:
                    second = transform(item.value) if item.value else None
                except AttributeError as error:
                    report(item.value)
                    report(error)
                    second = None
                records[item.key].value = parse(merged)
                records[item.key].filtered = second
    return records, output
",
        ),
        (
            "single_named_try_attribute_ifexp_control.py",
            r"def run(groups, allowed):
    output = {}
    records = {}
    for group_index, group in enumerate(groups):
        for item in group:
            if item.kind in allowed:
                if group_index not in output:
                    output[group_index] = []
                first = transform(item.value) if item.value else None
                output[group_index].append(Entry(value=item.value, filtered=first))
                if item.key not in records:
                    records[item.key] = Record(
                        before=None, after=None, weight=None,
                        key=item.key, value=None, latest=None,
                    )
                    records[item.key].before = Content(
                        value=load_before(item.value), key=item.key,
                    )
                records[item.key].latest = parse(item.value)
                records[item.key].after = Content(
                    value=load_after(item.value), key=item.key,
                )
                records[item.key].weight = measure(item.value)
                merged = combine(
                    records[item.key].before.value,
                    records[item.key].after.value,
                    item.key,
                )
                try:
                    second = transform(item.value) if item.value else None
                except AttributeError as error:
                    report(item.value)
                    report(error)
                    second = None
                records[item.key].value = parse(merged)
                records[item.key].filtered = second
    return records, output
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

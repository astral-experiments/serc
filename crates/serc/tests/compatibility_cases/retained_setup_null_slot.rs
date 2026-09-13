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

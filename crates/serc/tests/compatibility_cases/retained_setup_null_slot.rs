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

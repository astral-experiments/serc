use crate::common;

#[test]
fn matches_cpython_conditional_manager_suppression_path() {
    for (filename, source) in [
        (
            "conditional_nested_manager_return.py",
            r"def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                result = work(value)
                return result
        else:
            result = work(value)
            return result
",
        ),
        (
            "conditional_nested_manager_else_return.py",
            r"def convert(outer, inner, choose, value):
    with outer:
        if choose:
            return value
        else:
            with inner:
                return value
",
        ),
        (
            "conditional_nested_manager_raise.py",
            r"def convert(outer, inner, choose, error, value):
    with outer:
        if choose:
            with inner:
                raise error
        else:
            return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_conditional_manager_ownership() {
    for (filename, source) in [
        (
            "enclosing_try_conditional_manager.py",
            r"def convert(outer, inner, choose, value):
    try:
        with outer:
            if choose:
                with inner:
                    return value
            else:
                return value
    except Error:
        return None
",
        ),
        (
            "coroutine_conditional_sync_manager.py",
            r"async def convert(outer, inner, choose, value):
    with outer:
        if choose:
            with inner:
                return value
        else:
            return value
",
        ),
        (
            "outer_loop_conditional_manager.py",
            r"def convert(managers, inner, choose, value):
    for outer in managers:
        with outer:
            if choose:
                with inner:
                    return value
            else:
                return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_conditional_manager_controls() {
    for (filename, source) in [
        (
            "both_manager_arms.py",
            r"def convert(outer, left, right, choose, value):
    with outer:
        if choose:
            with left:
                return value
        else:
            with right:
                return value
",
        ),
        (
            "inner_manager_falls_through.py",
            r"def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                work(value)
        else:
            return value
",
        ),
        (
            "multiple_inner_manager_items.py",
            r"def convert(outer, inner, extra, choose, value):
    with outer:
        if choose:
            with inner, extra:
                return value
        else:
            return value
",
        ),
        (
            "async_inner_manager.py",
            r"async def convert(outer, inner, choose, value):
    with outer:
        if choose:
            async with inner:
                return value
        else:
            return value
",
        ),
        (
            "elif_manager_branch.py",
            r"def convert(outer, inner, first, second, value):
    with outer:
        if first:
            return value
        elif second:
            with inner:
                return value
        else:
            return value
",
        ),
        (
            "constant_manager_condition.py",
            r"def convert(outer, inner, value):
    with outer:
        if True:
            with inner:
                return value
        else:
            return value
",
        ),
        (
            "manager_condition_has_following_statement.py",
            r"def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                return value
        else:
            return value
        work(value)
",
        ),
        (
            "two_plain_terminal_arms.py",
            r"def convert(outer, choose, value):
    with outer:
        if choose:
            return value
        else:
            raise Error
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_conditional_finally_while_manager_exit() {
    common::assert_matches_cpython_marshal(
        r"def make_worker(processed, frequency):
    cursor = 0

    async def process(idx, manager, work):
        nonlocal cursor
        with manager():
            mark = False
            try:
                await work()
                mark = True
            except (Cancelled, Killed):
                raise
            except Exception:
                log()
                mark = True
            finally:
                if mark:
                    processed.add(idx)
                    while (cursor + 1) in processed:
                        cursor += 1
                        if cursor % frequency == 0:
                            heartbeat(cursor)
                            increment()

    return process
",
        "conditional_finally_while_manager.py",
    );
}

#[test]
fn matches_cpython_direct_finally_while_manager_exit() {
    common::assert_matches_cpython_marshal(
        r"def make_worker(processed, frequency):
    cursor = 0

    async def process(idx, manager, work):
        nonlocal cursor
        with manager():
            mark = False
            try:
                await work()
                mark = True
            except (Cancelled, Killed):
                raise
            except Exception:
                log()
                mark = True
            finally:
                processed.add(idx)
                while (cursor + 1) in processed:
                    cursor += 1
                    if cursor % frequency == 0:
                        heartbeat(cursor)
                        increment()

    return process
",
        "direct_finally_while_manager.py",
    );
}

#[test]
fn matches_cpython_shared_conditional_finally_manager_exit() {
    for (filename, source) in [
        (
            "shared_conditional_finally_manager_exit.py",
            r"def cleanup(outer, inner, work, exists, remove, value):
    with outer():
        resource = None
        try:
            resource = work(value)
        finally:
            if resource is not None and exists(resource):
                with inner():
                    remove(resource)
    return work(value)
",
        ),
        (
            "shared_conditional_finally_manager_three_edges.py",
            r"def cleanup(outer, inner, work, exists, remove, value, enabled):
    with outer():
        resource = None
        try:
            resource = work(value)
        finally:
            if resource is not None and enabled and exists(resource):
                with inner():
                    remove(resource)
    return work(value)
",
        ),
        (
            "located_chained_comparison_finally_manager_exit.py",
            r"def cleanup(outer, inner, work, remove, low, value, high):
    with outer():
        try:
            work(value)
        finally:
            if low < value < high:
                with inner():
                    remove(value)
    return work(value)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_retry_manager_suppression_owner() {
    for (filename, source) in [
        (
            "retry_conditional_manager_fallback.py",
            r"def read_source(source, retries):
    for attempt in range(retries + 1):
        try:
            if is_remote(source):
                with open_remote(source) as handle:
                    return handle.read()
            return read_local(source)
        except Exception as error:
            if attempt >= retries:
                raise
            pause(attempt)
    raise RuntimeError()
",
        ),
        (
            "retry_conditional_manager_branch_fallback.py",
            r"def read_source(source, retries):
    last_error = None
    for attempt in range(retries + 1):
        try:
            if is_remote(source):
                request = prepare(source)
                with open_remote(request) as handle:
                    return handle.read()
            if is_file(source):
                return read_file(source)
            return read_local(source)
        except Exception as error:
            last_error = error
            if attempt >= retries:
                break
            pause(attempt)
    raise last_error
",
        ),
        (
            "retry_direct_manager_loop_control.py",
            r"def read_source(source, retries):
    for attempt in range(retries + 1):
        try:
            with open_remote(source) as handle:
                return handle.read()
        except Exception as error:
            if attempt >= retries:
                raise
            pause(attempt)
    raise RuntimeError()
",
        ),
        (
            "retry_conditional_manager_break_control.py",
            r"def read_source(source, retries):
    for attempt in range(retries + 1):
        try:
            if is_remote(source):
                with open_remote(source) as handle:
                    return handle.read()
                break
            return read_local(source)
        except Exception as error:
            if attempt >= retries:
                raise
            pause(attempt)
    raise RuntimeError()
",
        ),
        (
            "retry_conditional_manager_fallthrough_control.py",
            r"def read_source(source, retries):
    for attempt in range(retries + 1):
        try:
            if is_remote(source):
                with open_remote(source) as handle:
                    consume(handle.read())
            return read_local(source)
        except Exception as error:
            if attempt >= retries:
                raise
            pause(attempt)
    raise RuntimeError()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_sync_manager_unique_normal_branch_donors() {
    for (filename, source) in [
        (
            "sync_manager_nested_if_unique_donor_target.py",
            r"def consume(open_task, stop_when_idle, work, pause):
    while True:
        with open_task() as task:
            if task is None:
                if stop_when_idle:
                    return
            else:
                work(task)
                continue
        pause()
",
        ),
        (
            "sync_manager_nested_if_two_normal_paths_control.py",
            r"def consume(open_task, stop_when_idle, work, pause):
    while True:
        with open_task() as task:
            if task is None:
                if stop_when_idle:
                    return
            else:
                work(task)
        pause()
",
        ),
        (
            "sync_manager_chained_pass_unique_donor_target.py",
            r"def stream(lock, read_batch, finished, wait):
    while True:
        with lock:
            entries = read_batch()
            if entries:
                pass
            elif finished():
                return
            else:
                wait()
                continue
        for entry in entries:
            yield entry
",
        ),
        (
            "sync_manager_chained_pass_two_normal_paths_control.py",
            r"def stream(lock, read_batch, finished, wait):
    while True:
        with lock:
            entries = read_batch()
            if entries:
                pass
            elif finished():
                return
            else:
                wait()
        for entry in entries:
            yield entry
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_manager_terminal_suppression_joins() {
    for (filename, source) in [
        (
            "nested_manager_literal_match_terminal.py",
            r#"async def run(self, outer, inner, read):
    with outer():
        with inner() as value:
            match self.kind:
                case "a":
                    return read(value, "a")
                case "b":
                    return read(value, "b")
                case "c":
                    return read(value, "c")
                case _:
                    raise RuntimeError("unknown kind")
"#,
        ),
        (
            "nested_manager_if_await_terminal.py",
            r"async def run(outer, inner, request, first, second):
    with outer():
        if first:
            if second is None:
                with inner() as value:
                    prepare(value)
                    return await request()
            else:
                return await request()
        else:
            return await request()
",
        ),
        (
            "nested_manager_pair_if_await_terminal.py",
            r"async def run(outer, other, inner, request, first, second):
    with (
        outer(),
        other(),
    ):
        if first:
            if second is None:
                with inner() as value:
                    prepare(value)
                    return await request()
            else:
                return await request()
        else:
            return await request()
",
        ),
        (
            "nested_manager_literal_match_nonexhaustive_control.py",
            r#"async def run(self, outer, inner, read):
    with outer():
        with inner() as value:
            match self.kind:
                case "a":
                    return read(value, "a")
                case "b":
                    return read(value, "b")
                case "c":
                    return read(value, "c")
"#,
        ),
        (
            "nested_manager_if_await_fallthrough_control.py",
            r"async def run(outer, inner, request, first, second):
    with outer():
        if first:
            if second is None:
                with inner() as value:
                    prepare(value)
                    return await request()
            else:
                await request()
        else:
            return await request()
",
        ),
        (
            "nested_manager_if_await_nonterminal_inner_control.py",
            r"async def run(outer, inner, request, first, second):
    with outer():
        if first:
            if second is None:
                with inner() as value:
                    prepare(value)
                    await request()
            else:
                return await request()
        else:
            return await request()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_manager_original_normal_exit_joins() {
    for (filename, source) in [
        (
            "manager_infinite_if_original_false_join_target.py",
            r"def f(manager, enabled, work):
    with manager():
        work()
        if enabled:
            while True:
                work()
",
        ),
        (
            "manager_terminal_if_return_unique_join_control.py",
            r"def f(manager, enabled, work):
    with manager():
        work()
        if enabled:
            return
",
        ),
        (
            "manager_nested_with_else_while_shared_join_target.py",
            r"def f(manager, inner, mode, pending, step, finish):
    with manager():
        if mode:
            with inner():
                while pending():
                    step()
        else:
            while pending():
                step()
    finish()
",
        ),
        (
            "manager_return_else_while_unique_join_control.py",
            r"def f(manager, mode, pending, step, finish):
    with manager():
        if mode:
            return
        else:
            while pending():
                step()
    finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn nested_raising_finally_keeps_manager_exit_marker() {
    for manager in ["with", "async with"] {
        for body in [
            "try:\n    raise ValueError\nexcept Exception as error:\n    try:\n        raise KeyError\n    finally:\n        cleanup()",
            "try:\n    try:\n        raise ValueError\n    finally:\n        await checkpoint()\nfinally:\n    try:\n        raise KeyError\n    finally:\n        cleanup()",
            "try:\n    raise ValueError\nexcept Exception:\n    cleanup()",
            "try:\n    if flag:\n        raise ValueError\nfinally:\n    cleanup()",
            "try:\n    raise ValueError\nfinally:\n    return result",
        ] {
            let body = body.replace('\n', "\n            ");
            let source = format!(
                "async def f(flag, result):
    with outer():
        {manager} inner():
            {body}
    finish()
"
            );
            common::assert_matches_cpython_marshal(&source, "nested_raising_manager_marker.py");
        }
    }
}

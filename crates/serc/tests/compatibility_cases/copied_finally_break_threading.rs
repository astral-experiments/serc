use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_break_threading_target() {
    let source = r#"def run(source_manager, temporary_manager, path_for, is_empty, upload, retry, finish, abort):
    completed = False
    try:
        with source_manager as source:
            part_number = 1
            while True:
                part_path = None
                try:
                    with temporary_manager as temporary:
                        part_path = path_for(temporary)
                        remaining = 3
                        while remaining:
                            chunk = source.read(remaining)
                            if not chunk:
                                break
                            temporary.write(chunk)
                            remaining -= len(chunk)
                    if is_empty(part_path):
                        break
                    result = upload(part_path, part_number)
                    if result != 0:
                        if retry(result):
                            return "RETRY"
                        raise RuntimeError(result)
                finally:
                    if part_path is not None:
                        part_path.unlink(missing_ok=True)
                part_number += 1
        finish()
        completed = True
        return "CREATED"
    finally:
        if not completed:
            abort()
"#;
    common::assert_matches_cpython_marshal(source, "copied_finally_break_target.py");
}

#[test]
fn matches_cpython_marshal_for_copied_finally_break_threading_after_finally_control() {
    let source = r#"def run(source_manager, temporary_manager, path_for, is_empty, upload, retry, finish, abort):
    completed = False
    try:
        with source_manager as source:
            part_number = 1
            while True:
                part_path = None
                stop = False
                try:
                    with temporary_manager as temporary:
                        part_path = path_for(temporary)
                        remaining = 3
                        while remaining:
                            chunk = source.read(remaining)
                            if not chunk:
                                break
                            temporary.write(chunk)
                            remaining -= len(chunk)
                    stop = is_empty(part_path)
                    if not stop:
                        result = upload(part_path, part_number)
                        if result != 0:
                            if retry(result):
                                return "RETRY"
                            raise RuntimeError(result)
                finally:
                    if part_path is not None:
                        part_path.unlink(missing_ok=True)
                if stop:
                    break
                part_number += 1
        finish()
        completed = True
        return "CREATED"
    finally:
        if not completed:
            abort()
"#;
    common::assert_matches_cpython_marshal(source, "after_finally_break_control.py");
}

#[test]
fn matches_cpython_marshal_for_conditional_finally_break_ordinary_continuations() {
    for (filename, source) in [
        (
            "conditional_finally_iterator_break_ordinary_continuation.py",
            r"def process(items, acquire, choose, finish):
    for item in items:
        reader = None
        try:
            reader = acquire(item)
            if choose(item):
                break
        finally:
            if reader is not None:
                reader.close()
    finish(reader)
    return reader
",
        ),
        (
            "conditional_finally_named_handler_break_ordinary_continuation.py",
            r"def process(items, acquire, choose, report, finish):
    reader = None
    for item in items:
        try:
            reader = acquire(item)
            if choose(item):
                break
        except Exception as error:
            report(error)
            break
        finally:
            if reader is not None:
                reader.close()
    finish(reader)
    return reader
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_guarded_try_finally_iterator_breaks() {
    for (filename, source) in [
        (
            "guarded_body_and_handler_break_none_guard.py",
            r"def connect(resources, limit, ping):
    last_error = None
    for attempt, resource in enumerate(resources):
        try:
            ping(resource)
            last_error = None
            break
        except Exception as error:
            last_error = error
            if attempt == limit:
                break
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    if last_error is not None:
        raise last_error
    return True
",
        ),
        (
            "guarded_continue_cleanup_control.py",
            r"def visit(resources, perform):
    count = 0
    for resource in resources:
        try:
            if perform(resource):
                continue
            count += 1
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    return count
",
        ),
        (
            "guarded_handler_break_call_guard.py",
            r"def transfer(resource, attempts, limit, perform, already_done):
    last_error = None
    for attempt in attempts:
        try:
            perform()
            return
        except Exception as error:
            if already_done(error):
                return
            last_error = error
            if attempt == limit:
                break
        finally:
            if resource.exists():
                try:
                    resource.close()
                except OSError:
                    pass
    assert last_error is not None
    raise last_error
",
        ),
        (
            "guarded_return_cleanup_control.py",
            r"def first(resources):
    for resource in resources:
        try:
            return resource
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_async_generator_guarded_finally_break_owners() {
    for (filename, source) in [
        (
            "async_generator_named_break_guarded_global_await_target.py",
            r"async def run(items, make, retry):
    for item in items:
        handle = None
        try:
            handle = await make(item)
            yield handle
            return
        except Exception as error:
            if retry(error):
                continue
            break
        finally:
            if handle is not None:
                await close_handle(handle)
    raise RuntimeError
",
        ),
        (
            "async_generator_named_break_unguarded_global_await_control.py",
            r"async def run(items, make, retry):
    for item in items:
        handle = None
        try:
            handle = await make(item)
            yield handle
            return
        except Exception as error:
            if retry(error):
                continue
            break
        finally:
            await close_handle(handle)
    raise RuntimeError
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_plain_conjunction_finally_iterator_break_boundary() {
    let source = r"def collect(items, ready, failed, callback):
    for item in items:
        try:
            if item:
                break
        finally:
            if ready and not failed and callback is not None:
                callback(item, ready)
    return items
";
    common::assert_matches_cpython_marshal(
        source,
        "conjunction_finally_iterator_break_plain_target.py",
    );
}

#[test]
fn matches_cpython_shared_iterator_cleanup_join_copies() {
    for (filename, source) in [
        (
            "p280_named_finally_conditional_cleanup.py",
            r#"def run(items, manager, work, recover, stop, cleanup, keep):
    with manager:
        state = None
        for item in items:
            try:
                state = work(item)
                break
            except LookupError as error:
                state = recover(error)
                if stop(error):
                    break
            finally:
                if state and not keep:
                    try:
                        cleanup(state)
                    except ValueError as cleanup_error:
                        if state is not None:
                            if state.error is None:
                                state.error = f"cleanup: {cleanup_error}"
                            else:
                                state.error = f"{state.error}; cleanup: {cleanup_error}"
    return state
"#,
        ),
        (
            "p280_shared_finally_iterator_join.py",
            r"def run(items, work, recover, stop, cleanup, keep):
    state = None
    for item in items:
        try:
            state = work(item)
            break
        except LookupError as error:
            state = recover(error)
            if stop(error):
                break
        finally:
            if state and not keep:
                try:
                    cleanup(state)
                except ValueError:
                    pass
    return state
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_copied_finally_global_call_array_boundaries() {
    for (filename, source) in [
        (
            "named_finally_split_six_global_prefix_target.py",
            r#"def run(items, manager, recover, stop, cleanup, keep, clock,
        flag, work):
    with manager:
        state = None
        for item in items:
            try:
                if flag:
                    work(item)
                state = Record(
                    first=str(fetch(item)),
                    second=str(classify(item)),
                    elapsed=round(clock.now(), 3),
                )
                break
            except LookupError as error:
                state = recover(error)
                if stop(error):
                    break
            finally:
                if state and not keep:
                    try:
                        cleanup(state)
                    except ValueError as cleanup_error:
                        if state is not None:
                            if state.error is None:
                                state.error = f"cleanup: {cleanup_error}"
                            else:
                                state.error = f"{state.error}; cleanup: {cleanup_error}"
    return state
"#,
        ),
        (
            "named_finally_linear_six_global_prefix_control.py",
            r#"def run(items, manager, recover, stop, cleanup, keep, clock,
        flag, work):
    with manager:
        state = None
        for item in items:
            try:
                work(item)
                state = Record(
                    first=str(fetch(item)),
                    second=str(classify(item)),
                    elapsed=round(clock.now(), 3),
                )
                break
            except LookupError as error:
                state = recover(error)
                if stop(error):
                    break
            finally:
                if state and not keep:
                    try:
                        cleanup(state)
                    except ValueError as cleanup_error:
                        if state is not None:
                            if state.error is None:
                                state.error = f"cleanup: {cleanup_error}"
                            else:
                                state.error = f"{state.error}; cleanup: {cleanup_error}"
    return state
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_shared_copied_finally_break_conditions() {
    for (filename, source) in [
        (
            "same_head_two_sync.py",
            r"def process(self, work, active, response):
    while active:
        try:
            try:
                response = work()
            except ConnectionError:
                break
            if response is None:
                break
            self.consume(response)
        finally:
            self.task = None
            if self.transport is None and response is not None:
                self.report(response)
        self.advance()
    self.finish()
    return response
",
        ),
        (
            "same_head_two_async.py",
            r"async def process(self, work, active, response):
    while active:
        try:
            try:
                response = await work()
            except ConnectionError:
                break
            if response is None:
                break
            self.consume(response)
        finally:
            self.task = None
            if self.transport is None and response is not None:
                self.report(response)
        self.advance()
    self.finish()
    return response
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

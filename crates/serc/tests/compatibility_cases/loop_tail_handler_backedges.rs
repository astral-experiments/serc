use crate::common;

#[test]
fn matches_cpython_loop_tail_handler_backedges() {
    for prefix in ["", "async "] {
        for binding in ["", " as error"] {
            for returns in [false, true] {
                for loop_header in ["for item in items:", "while ready:"] {
                    for handler_tail in [
                        "if ready:\n    recover()",
                        "if ready:\n    recover()\nelse:\n    fallback()",
                        "if ready:\n    return recover()",
                    ] {
                        let returned = if returns { "return " } else { "" };
                        let awaited = if prefix.is_empty() { "" } else { "await " };
                        let source = format!(
                            "{prefix}def f(items, ready):\n    {loop_header}\n        try:\n            {returned}{awaited}work(item)\n        except Error{binding}:\n            record()\n            {}\n    finish()\n",
                            handler_tail.replace('\n', "\n            "),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "loop_tail_handler_backedges.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_awaited_loop_tail_handler_backedges() {
    for binding in ["", " as error"] {
        for returns in [false, true] {
            for loop_header in ["for item in items:", "while ready:"] {
                let returned = if returns { "return " } else { "" };
                let source = format!(
                    "async def f(items, ready):\n    {loop_header}\n        try:\n            {returned}await work(item)\n        except Error{binding}:\n            record()\n            if ready:\n                await recover()\n    finish()\n",
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "awaited_loop_tail_handler_backedges.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_loop_tail_handler_backedges_with_else() {
    for prefix in ["", "async "] {
        for loop_header in ["for item in items:", "while ready:"] {
            for binding in ["", " as error"] {
                for handler_tail in [
                    "if ready:\n    recover(item)",
                    "if ready:\n    recover(item)\nelse:\n    fallback(item)",
                ] {
                    for normal_tail in [
                        "completed(item)",
                        "return item",
                        "if ready:\n    completed(item)",
                    ] {
                        let awaited = if prefix.is_empty() { "" } else { "await " };
                        let source = format!(
                            "{prefix}def f(items, ready):\n    {loop_header}\n        try:\n            result = {awaited}work(item)\n        except Error{binding}:\n            record()\n            {}\n        else:\n            {}\n    finish()\n",
                            handler_tail.replace('\n', "\n            "),
                            normal_tail.replace('\n', "\n            "),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "loop_tail_handler_backedges_with_else.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_shared_handler_interrupt_checks() {
    common::assert_matches_cpython_marshal(
        r"async def f(stop, wait, record, value):
    try:
        while True:
            try:
                await wait(stop)
                break
            except TimeoutError:
                value = stop.current()
                if value is not None:
                    record(value)
    except RuntimeError:
        pass
",
        "async_unnamed_loop.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(stop, wait, record, value):
    try:
        while True:
            try:
                wait(stop)
                break
            except TimeoutError:
                value = stop.current()
                if value is not None:
                    record(value)
    except RuntimeError:
        pass
",
        "sync_unnamed_loop.py",
    );
}

#[test]
fn matches_cpython_shared_protected_loop_interrupt_checks() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        try:
            if item is None:
                continue
            else:
                consume(item)
                if ready(item):
                    emit(item)
        except Exception as exc:
            recover(exc)
",
        "protected_else_shared_tail.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(manager, items, limit=None):
    count = 0
    seen = 0
    errors = 0
    with manager as stream:
        for item in items:
            if limit is not None and seen >= limit:
                break
            seen += 1
            try:
                result = process(item)
                if result is None:
                    skipped(item)
                    errors += 1
                    continue
                else:
                    stream.write(result)
                    count += 1
                    if count % 100 == 0:
                        report(count)
            except Exception as exc:
                recover(exc)
    complete(errors)
",
        "manager_protected_else_shared_tail.py",
    );
}

#[test]
fn copied_assertion_backedges_leave_their_original_try() {
    for (source, filename) in [
        (
            "def f(items, debug, flag):\n    for item in items:\n        try:\n            if not debug:\n                try:\n                    work(item)\n                    if flag:\n                        consume(item)\n                except ValueError as error:\n                    report(error)\n                    pass\n        except RuntimeError as error:\n            report(error)\n            pass\n",
            "nested_try_tail_backedges.py",
        ),
        (
            "def f(items, verify, strict):\n    for item in items:\n        try:\n            work(item)\n            if verify:\n                if strict:\n                    assert check(item)\n                else:\n                    assert other(item)\n        except ValueError as error:\n            report(error)\n",
            "nested_assert_tail_backedges.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn final_else_copied_backedges_leave_nested_tries() {
    for (source, filename) in [
        (
            "def f(items, debug, flag):\n    for item in items:\n        try:\n            if not debug:\n                try:\n                    work(item)\n                    if flag:\n                        consume(item)\n                except ValueError as error:\n                    report(error)\n                    pass\n        except RuntimeError as error:\n            report(error)\n            pass\n",
            "nested_try_tail_backedges.py",
        ),
        (
            "def f(items, debug, flag):\n    for item in items:\n        try:\n            if not debug:\n                consume(item)\n            else:\n                try:\n                    work(item)\n                    if flag:\n                        consume(item)\n                except ValueError as error:\n                    report(error)\n                    pass\n        except RuntimeError as error:\n            report(error)\n            pass\n",
            "nested_try_final_else_backedges.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_normal_try_else_loop_tail_backedges() {
    for (source, filename) in [
        (
            r#"def collect(widgets, decode):
    first = set()
    second = set()
    for widget in widgets:
        if widget.bindings is not None:
            for binding in widget.bindings.values():
                first.add(binding.identifier)
                if binding.version is not None:
                    second.add(binding.version)
        elif widget.kind == "form":
            try:
                value = decode(widget.output)
            except Exception:
                pass
            else:
                if value.identifier is not None:
                    first.add(value.identifier)
                if value.version is not None:
                    second.add(value.version)
    return first, second
"#,
            "try_else_outer_elif_two_conditions_target.py",
        ),
        (
            r"async def check(cases, fetch, decode):
    failures = []
    for case in cases:
        path = case.path
        if isinstance(path, str) and path.strip():
            ok, output = await fetch(path)
            if not ok:
                failures.append(output)
            else:
                try:
                    parsed = decode(output)
                except ValueError as exc:
                    failures.append(str(exc))
                else:
                    if not isinstance(parsed, list):
                        failures.append(type(parsed).__name__)
    return failures
",
            "try_else_async_nested_named_recovery_target.py",
        ),
        (
            r#"async def drain(source, timeout):
    while True:
        reservation = await source.reserve()
        if not isinstance(reservation, str):
            await source.pause()
            continue
        if source.enabled:
            try:
                async with timeout():
                    result = await source.ack(reservation)
            except Exception:
                source.failures += 1
                source.report_failure()
            else:
                if isinstance(result, dict) and result.get("acked") is True:
                    source.acked += 1
"#,
            "try_else_async_manager_short_circuit_target.py",
        ),
        (
            r"def with_finally(items, output):
    for item in items:
        try:
            value = item.read()
        except ValueError:
            pass
        else:
            if value:
                output.append(value)
        finally:
            item.close()
    return output


def folded_tail(items, output):
    for item in items:
        try:
            value = item.read()
        except ValueError:
            pass
        else:
            if False:
                output.append(value)
    return output
",
            "try_else_backedge_boundaries_control.py",
        ),
        (
            r"async def publish(self):
    retry_delay = self.base_delay
    retry_attempt = 1
    while not self.closed:
        await self.pending.wait()
        if self.closed:
            return
        incremental_delay = self.remaining_delay()
        if incremental_delay > 0:
            await pause(incremental_delay)
            if self.closed:
                return
        try:
            published = await self.flush(retry_attempt=retry_attempt)
        except Cancelled:
            raise
        except Superseded as exc:
            retry_delay = self.base_delay
            retry_attempt = 1
            if self.generation == exc.generation:
                report(exc.generation, self.name)
                self.abandon()
        except Exception as exc:
            report(self.name, exc, exc_info=exc)
            self.notify()
            await pause(jitter(retry_delay))
            retry_attempt += 1
            retry_delay = min(self.max_delay, max(self.base_delay, retry_delay * 2))
        else:
            retry_delay = self.base_delay
            retry_attempt = 1
            if not published:
                self.pending.clear()
",
            "async_handler_and_normal_else_backedges_target.py",
        ),
        (
            r"async def publish(self):
    retry_delay = self.base_delay
    retry_attempt = 1
    while not self.closed:
        await self.pending.wait()
        if self.closed:
            return
        incremental_delay = self.remaining_delay()
        if incremental_delay > 0:
            await pause(incremental_delay)
            if self.closed:
                return
        with self.manager:
            try:
                published = await self.flush(retry_attempt=retry_attempt)
            except Cancelled:
                raise
            except Superseded as exc:
                retry_delay = self.base_delay
                retry_attempt = 1
                if self.generation == exc.generation:
                    report(exc.generation, self.name)
                    self.abandon()
            except Exception as exc:
                report(self.name, exc, exc_info=exc)
                self.notify()
                await pause(jitter(retry_delay))
                retry_attempt += 1
                retry_delay = min(self.max_delay, max(self.base_delay, retry_delay * 2))
            else:
                retry_delay = self.base_delay
                retry_attempt = 1
                if not published:
                    self.pending.clear()
",
            "async_handler_and_normal_else_manager_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_protected_return_false_backedges_after_recovery() {
    for (source, filename) in [
        (
            r"def f(self):
    while True:
        try:
            try:
                item = self.queue.get()
                if item is not None:
                    self.buffer.append(item)
            except Empty:
                pass
            if self.ready:
                self.flush()
            if self.closed and self.queue.empty():
                return
        except Exception as error:
            report(error)
            if self.closed:
                return
            else:
                pause()
",
            "protected_terminal_and_return_backedges_target.py",
        ),
        (
            r"def f(self):
    while True:
        try:
            with self.lock:
                count = len(self.buffer)
            try:
                item = self.queue.get()
                if item is not None:
                    with self.lock:
                        self.buffer.append(item)
            except Empty:
                pass
            if self.ready:
                self.flush()
            if self.closed and self.queue.empty():
                return
        except Exception as error:
            report(error)
            if self.closed:
                return
            else:
                pause()
",
            "protected_terminal_and_return_manager_prefix_target.py",
        ),
        (
            r"def f(self):
    while True:
        try:
            try:
                item = self.queue.get()
                if item is not None:
                    self.buffer.append(item)
            except Empty:
                pass
            if self.ready:
                self.flush()
            if self.closed:
                return
        except Exception as error:
            report(error)
            if self.closed:
                return
            else:
                pause()
",
            "protected_terminal_single_return_backedge_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

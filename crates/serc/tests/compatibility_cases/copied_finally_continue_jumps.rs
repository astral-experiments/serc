use crate::common;

#[test]
fn matches_cpython_copied_finally_continue_handler_jumps() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield values\n"),
    ] {
        for loop_header in ["for value in values", "while flag"] {
            for cleanup in [
                "try:\n    cleanup(value)\nexcept Error:\n    pass",
                "try:\n    cleanup(value)\nexcept Error as error:\n    pass",
                "try:\n    cleanup(value)\nexcept:\n    pass",
                "try:\n    cleanup(value)\nexcept Error:\n    failed(value)",
                "try:\n    cleanup(value)\nexcept Error:\n    pass\nelse:\n    success(value)",
            ] {
                let cleanup = cleanup.replace('\n', "\n            ");
                let source = format!(
                    "{definition} f(values, flag, manager):\n    {loop_header}:\n        try:\n            consume(value)\n            continue\n        finally:\n            {cleanup}\n{footer}    return values\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_finally_continue_handler.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_copied_finally_continue_nested_cleanup() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield values\n"),
    ] {
        for loop_header in ["for value in values", "while flag"] {
            for cleanup in [
                "try:\n    cleanup(value)\nfinally:\n    try:\n        other(value)\n    except Error:\n        pass",
                "try:\n    cleanup(value)\nexcept Error:\n    pass\nfinally:\n    other(value)",
                "with manager:\n    try:\n        cleanup(value)\n    except Error:\n        pass",
                "for nested in values:\n    try:\n        cleanup(nested)\n    except Error:\n        pass",
            ] {
                let cleanup = cleanup.replace('\n', "\n            ");
                let source = format!(
                    "{definition} f(values, flag, manager):\n    {loop_header}:\n        try:\n            consume(value)\n            continue\n        finally:\n            {cleanup}\n{footer}    return values\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_finally_continue_nested.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_continue_stays_inside_its_finally_context() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield values\n"),
    ] {
        for loop_header in ["for value in values", "while flag"] {
            for body in [
                "try:\n    for nested in values:\n        consume(value)\n        continue\nfinally:\n    try:\n        cleanup(value)\n    except Error:\n        pass",
                "try:\n    consume(value)\nfinally:\n    for nested in values:\n        consume(value)\n        continue",
            ] {
                let body = body.replace('\n', "\n        ");
                let source = format!(
                    "{definition} f(values, flag, manager):\n    {loop_header}:\n        {body}\n{footer}    return values\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "continue_within_finally_context.py",
                );
            }
        }
    }
}

#[test]
fn copied_finally_none_continue_heads_match_cpython() {
    for (filename, source) in [
        (
            "while_is_not_none.py",
            r"async def consume(items, active, resource, work, close, notify, done):
    while active:
        try:
            await work(resource)
        except Exception as exc:
            notify(exc)
            continue
        finally:
            if resource is not None:
                try:
                    await close(resource)
                except Exception as error:
                    notify(error)
    return resource
",
        ),
        (
            "while_is_none.py",
            r"async def consume(items, active, resource, work, close, notify, done):
    while active:
        try:
            await work(resource)
        except Exception as exc:
            notify(exc)
            continue
        finally:
            if resource is None:
                try:
                    await close(resource)
                except Exception as error:
                    notify(error)
    return resource
",
        ),
        (
            "for_name.py",
            r"async def consume(items, active, resource, work, close, notify, done):
    for unused in items:
        try:
            await work(resource)
        except Exception as exc:
            notify(exc)
            continue
        finally:
            if resource:
                try:
                    await close(resource)
                except Exception as error:
                    notify(error)
    return resource
",
        ),
        (
            "while_is_not_none_suffix_control.py",
            r"async def consume(items, active, resource, work, close, notify, done):
    while active:
        try:
            await work(resource)
        except Exception as exc:
            notify(exc)
            continue
        finally:
            if resource is not None:
                try:
                    await close(resource)
                except Exception as error:
                    notify(error)
            done(resource)
    return resource
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn copied_finally_attribute_continue_owners_and_borrowing_match_cpython() {
    for (filename, source) in [
        (
            "for_continue_attribute_none_followed_call.py",
            r"async def f(items):
    for item in items:
        try:
            work(item)
        except Error as error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None:
                close(item)
            touch(item)
",
        ),
        (
            "awaited_while_continue_attribute_none_and_local_followed_call.py",
            r"async def f(items):
    while True:
        item = await items.get()
        retained = False
        try:
            work(item)
        except Error as error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None and not retained:
                close(item)
            touch(item)
",
        ),
        (
            "for_continue_attribute_none_followed_assignment.py",
            r"async def f(items):
    for item in items:
        try:
            work(item)
        except Error as error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None:
                close(item)
            item.closed = True
",
        ),
        (
            "worker_continue_attribute_none_full_shape.py",
            r#"async def f(self):
    while True:
        item = await self.queue.get()
        self.busy = True
        retained = False
        try:
            if self.active is not None:
                try:
                    await runtime.shield(self.active)
                except Retry:
                    pass
            async with self.lock:
                await self.start(
                    item.input,
                    item.key,
                    item.destination,
                    item.bound,
                    item.fresh,
                    item.progress,
                )
                if item.resource is not None:
                    self.retain(item.resource)
                    retained = True
                task = self.active
            if task is None:
                raise RuntimeError("missing task")
            response = await runtime.shield(task)
            if not item.future.done():
                item.future.set_result(response)
        except runtime.Cancelled:
            if not item.future.done():
                item.future.cancel()
            raise
        except Exception as error:
            if not item.future.done():
                item.future.set_exception(error)
            if isinstance(error, Retry):
                continue
            if self.failure is not None and not self.failure.done():
                self.failure.set_exception(error)
            return
        finally:
            if item.resource is not None and not retained:
                item.resource.cleanup()
            self.busy = False
            self.queue.task_done()
"#,
        ),
        (
            "named_continue_attribute_none_and_guard.py",
            r"async def f(items):
    for item in items:
        try:
            work(item)
        except Error as error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None and item.ready:
                close(item)
",
        ),
        (
            "unnamed_continue_attribute_control.py",
            r"async def f(items):
    for item in items:
        try:
            work(item)
        except Error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None:
                close(item)
",
        ),
        (
            "empty_try_source_attribute_control.py",
            r"async def f(items):
    for item in items:
        try:
            pass
        except Error as error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None:
                close(item)
            touch(item)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

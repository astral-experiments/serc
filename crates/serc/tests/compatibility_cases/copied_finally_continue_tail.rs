use crate::common;

#[test]
fn normalizes_conditional_finally_copies_before_continue() {
    for (definition, prefix) in [("def", ""), ("async def", ""), ("def", "    yield None\n")] {
        for loop_header in ["for item in items", "while flag"] {
            for selector in ["other", "not other"] {
                for protected_body in [
                    "        try:\n            continue\n",
                    "        try:\n            use()\n        except ValueError:\n            continue\n",
                ] {
                    let source = format!(
                        "{definition} f(items, flag, other):\n{prefix}    {loop_header}:\n{protected_body}        finally:\n            if {selector}:\n                cleanup()\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "copied_finally_continue_tail.py",
                    );
                }
            }
        }
    }
}

#[test]
fn normalizes_continue_tail_when_try_encloses_the_whole_loop() {
    for (definition, prefix) in [("def", ""), ("async def", ""), ("def", "    yield None\n")] {
        for loop_header in ["for item in items", "while other"] {
            let source = format!(
                "{definition} f(items, flag, other):\n{prefix}    try:\n        {loop_header}:\n            try:\n                work()\n            except LookupError:\n                continue\n            finally:\n                if flag:\n                    action()\n    except BaseException:\n        recover()\n"
            );
            common::assert_matches_cpython_marshal(&source, "copied_finally_enclosing_loop.py");
        }
    }
}

#[test]
fn continued_finally_pop_records_preserve_coroutine_marker_owners() {
    for (filename, source) in [
        (
            "async_continue_after_suppress_before_finally_cancel.py",
            r#"async def read(self):
    if not self.enabled:
        return await self.primary.read()
    while True:
        if not self.selected:
            primary_read = asyncio.create_task(self.primary.read())
            activation_wait = asyncio.create_task(self.activation.wait())
            try:
                done, _ = await asyncio.wait(
                    {primary_read, activation_wait},
                    return_when=asyncio.FIRST_COMPLETED,
                )
                if activation_wait in done and self.selected:
                    if primary_read.done():
                        with suppress(asyncio.CancelledError, Exception):
                            primary_read.result()
                    continue
                chunk = await primary_read
                return Packet(
                    data=chunk.data,
                    time=chunk.time,
                    speaker=chunk.speaker,
                    enabled=chunk.enabled,
                    epoch=chunk.epoch,
                    generation=chunk.generation,
                    source=chunk.source or "primary",
                )
            finally:
                if not primary_read.done():
                    primary_read.cancel()
                    with suppress(asyncio.CancelledError):
                        await primary_read
                if not activation_wait.done():
                    activation_wait.cancel()
                    with suppress(asyncio.CancelledError):
                        await activation_wait
        return await self.fallback.read()
"#,
        ),
        (
            "continued_finally_counter.py",
            r#"async def continued_finally_counter(queue, totals, interval):
    processed = 0
    cache = {}
    while True:
        item = await queue.get()
        if item is None:
            queue.done()
            break
        processed += 1
        try:
            if item.key:
                if item.key in cache:
                    value = cache[item.key]
                else:
                    try:
                        value = await fetch(item.key)
                    except Error as exc:
                        report("lookup", extra={"key": item.key, "error": str(exc)})
                        value = None
                    cache[item.key] = value
                if value is None:
                    totals["skipped"] = totals.get("skipped", 0) + 1
                    continue
                async with context(value):
                    status = await process(item=item)
            else:
                status = await process(item=item)
            totals[status] = totals.get(status, 0) + 1
        except Error as exc:
            report("process", extra={"item": str(item.key), "error": str(exc)})
            totals["errors"] = totals.get("errors", 0) + 1
        finally:
            if interval and processed % interval == 0:
                report("progress", extra={"processed": processed, "totals": dict(totals)})
            queue.done()
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn continued_finally_pop_records_preserve_ordinary_for_marker_owners() {
    for (filename, source) in [
        (
            "continued_finally_timing.py",
            r#"async def continued_finally_timing(channels):
    for channel in channels:
        tags = channel_tags(channel)
        started = clock()
        count = 0
        successes = 0
        failures = 0
        try:
            items = await fetch(channel)
            if len(items) == 0:
                report("empty")
                continue
            count = len(items)
            results = await process(items)
            for result in results:
                if isinstance(result, RecoverableError):
                    failures += 1
                    await record_failure(result)
                elif isinstance(result, Exception):
                    raise result
                else:
                    successes += 1
            await flush(channel)
        except RecoverableError as exc:
            report("failed", extra={"error": str(exc)})
            await record_failure(exc)
        finally:
            elapsed = clock() - started
            if count:
                distribution("count", count, tags=tags)
            if successes:
                increment("success", successes, tags=tags)
            if failures:
                increment("failure", failures, tags=tags)
            increment("batches", tags=tags)
            distribution("elapsed", elapsed, tags=tags)
"#,
        ),
        (
            "continued_finally_timing_attributes.py",
            r#"async def continued_finally_timing(channels):
    for channel in channels:
        tags = channel_tags(channel)
        started = time.perf_counter()
        count = 0
        successes = 0
        failures = 0
        try:
            items = await fetch(channel)
            if len(items) == 0:
                logger.info("empty")
                continue
            count = len(items)
            results = await process(items)
            for result in results:
                if isinstance(result, RecoverableError):
                    failures += 1
                    await record_failure(result)
                elif isinstance(result, Exception):
                    raise result
                else:
                    successes += 1
            await flush(channel)
        except RecoverableError as exc:
            report("failed", extra={"error": str(exc)})
            await record_failure(exc)
        finally:
            elapsed = time.perf_counter() - started
            if count:
                distribution("count", count, tags=tags)
            if successes:
                increment("success", successes, tags=tags)
            if failures:
                increment("failure", failures, tags=tags)
            increment("batches", tags=tags)
            distribution("elapsed", elapsed, tags=tags)
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

async def continued_finally_timing(channels):
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

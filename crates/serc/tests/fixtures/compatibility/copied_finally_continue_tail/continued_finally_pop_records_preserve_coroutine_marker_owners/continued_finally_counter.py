async def continued_finally_counter(queue, totals, interval):
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

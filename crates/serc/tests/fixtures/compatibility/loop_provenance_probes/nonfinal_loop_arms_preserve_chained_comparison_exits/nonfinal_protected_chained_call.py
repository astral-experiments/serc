def target(items, enabled, low, high, consume):
    try:
        for item in items:
            if enabled:
                if low <= item < high:
                    consume(item)
            else:
                consume(None)
    except Exception:
        consume(None)

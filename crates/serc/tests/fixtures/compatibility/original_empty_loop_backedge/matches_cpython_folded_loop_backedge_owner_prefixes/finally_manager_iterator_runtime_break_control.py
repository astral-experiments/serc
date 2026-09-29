def run(manager, items, limit, ready, cleanup, fallback):
    try:
        with manager:
            for item in items:
                if ready(item) if limit is not None else fallback(item):
                    break
    finally:
        cleanup()

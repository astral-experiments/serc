def run(manager, items, limit, ready, cleanup):
    try:
        with manager:
            for item in items:
                if ready(item) if limit is not None else False:
                    break
    finally:
        cleanup()

def run(items, manager, ready, cleanup):
    try:
        for item in items:
            with manager:
                if ready(item):
                    return None
    finally:
        cleanup()

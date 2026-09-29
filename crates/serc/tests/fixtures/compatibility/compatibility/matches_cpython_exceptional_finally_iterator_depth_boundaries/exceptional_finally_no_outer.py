def run(source, enabled, pending):
    try:
        return pending
    finally:
        if enabled:
            for item in source:
                return item

def sync_generator(items):
    try:
        for item in items:
            if item:
                break
        yield item
    finally:
        cleanup()

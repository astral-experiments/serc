def cleanup(items, manager, acquire, finish):
    for item in items:
        with manager:
            resource = None
            try:
                resource = acquire(item)
                resource.start()
            except Exception:
                continue
            finally:
                if resource:
                    resource.close(wait=True)
            finish(item)

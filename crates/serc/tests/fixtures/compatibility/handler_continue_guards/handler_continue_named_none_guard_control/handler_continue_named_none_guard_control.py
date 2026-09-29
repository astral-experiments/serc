def cleanup(items, manager, acquire, finish):
    for item in items:
        with manager:
            resource = None
            try:
                resource = acquire(item)
                resource.start()
            except Exception as error:
                continue
            finally:
                if resource is not None:
                    resource.close(wait=True)
            finish(item)

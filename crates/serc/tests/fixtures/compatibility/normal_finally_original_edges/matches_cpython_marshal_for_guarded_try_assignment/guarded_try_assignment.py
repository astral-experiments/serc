def guarded_try(resource, work):
    try:
        work()
    finally:
        if resource is not None:
            try:
                resource.close()
            except Exception:
                pass
            resource = None

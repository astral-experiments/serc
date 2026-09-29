def cleanup(outer, inner, work, exists, remove, value):
    with outer():
        resource = None
        try:
            resource = work(value)
        finally:
            if resource is not None and exists(resource):
                with inner():
                    remove(resource)
    return work(value)

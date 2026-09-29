def cleanup(outer, inner, work, exists, remove, value, enabled):
    with outer():
        resource = None
        try:
            resource = work(value)
        finally:
            if resource is not None and enabled and exists(resource):
                with inner():
                    remove(resource)
    return work(value)

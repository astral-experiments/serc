def first(resources):
    for resource in resources:
        try:
            return resource
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    return None

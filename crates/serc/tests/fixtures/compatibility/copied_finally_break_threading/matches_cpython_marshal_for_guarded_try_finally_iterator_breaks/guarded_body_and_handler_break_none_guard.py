def connect(resources, limit, ping):
    last_error = None
    for attempt, resource in enumerate(resources):
        try:
            ping(resource)
            last_error = None
            break
        except Exception as error:
            last_error = error
            if attempt == limit:
                break
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    if last_error is not None:
        raise last_error
    return True

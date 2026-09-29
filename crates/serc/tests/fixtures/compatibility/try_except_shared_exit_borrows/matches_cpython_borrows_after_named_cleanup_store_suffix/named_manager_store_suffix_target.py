def lease(resource, acquire, manager, report):
    payload = resource.payload
    try:
        handle = acquire(resource)
    except OSError as error:
        existing = resource.read()
        if existing.active:
            if existing.owner:
                try:
                    yield existing
                finally:
                    try:
                        current = resource.read()
                        if current.active and current.owner:
                            if existing.restore:
                                report(existing)
                            else:
                                with manager():
                                    resource.remove()
                    finally:
                        report(resource)
                return
            reason = existing.reason
            raise RuntimeError(reason) from error
        with manager():
            resource.remove()
        handle = acquire(resource)
    with resource.open(handle) as output:
        output.write(payload)
    report(resource)
    try:
        yield handle
    finally:
        try:
            current = resource.read()
            if current.active and current.owner:
                with manager():
                    resource.remove()
        finally:
            report(resource)

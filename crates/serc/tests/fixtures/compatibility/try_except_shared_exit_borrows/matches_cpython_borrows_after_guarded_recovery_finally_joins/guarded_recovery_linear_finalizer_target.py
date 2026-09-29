def collect(resource, work, recover, report, enabled, retry):
    try:
        try:
            value = work(resource)
            if enabled:
                report(value)
        except Exception:
            if not retry:
                raise
            fallback = recover(resource)
            try:
                work(fallback)
                report(fallback)
            finally:
                fallback.close(missing_ok=True)
    finally:
        resource.close(missing_ok=True)
    report(resource)
    report(enabled, retry)
    return resource

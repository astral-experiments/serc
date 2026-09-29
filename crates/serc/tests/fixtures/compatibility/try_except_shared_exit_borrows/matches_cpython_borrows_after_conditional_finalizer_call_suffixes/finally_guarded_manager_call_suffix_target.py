def finish(work, worker, resource, connect, endpoint, report):
    value = None
    try:
        value = work(endpoint)
    finally:
        if worker.is_alive():
            try:
                with connect(endpoint, timeout=1):
                    pass
            except OSError:
                pass
        worker.join(timeout=5)
        resource.close()
    if worker.is_alive():
        raise RuntimeError("worker active")
    if value is None:
        raise RuntimeError("missing result")
    report(value, endpoint)
    return value

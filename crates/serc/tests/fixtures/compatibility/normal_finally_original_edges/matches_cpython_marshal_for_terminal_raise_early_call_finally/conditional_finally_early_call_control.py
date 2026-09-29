def finish_job(start, check, worker_name):
    result = {"error": None}
    worker: object | None = None
    stop_event: object | None = None
    try:
        worker, stop_event = start(worker_name)
        original = worker.settings()
        try:
            worker.configure()
            observed = worker.settings()
        finally:
            worker.restore(original)
        restored = worker.settings()
        if not check(observed, restored):
            raise Error("unexpected state")
    except Error as error:
        result["error"] = format_error(error)
    finally:
        cleanup_started()
        if stop_event is not None:
            stop_event.set()
        if worker is not None:
            worker.join(timeout=5)
            if worker.is_alive():
                previous = result["error"]
                result["error"] = "still running" if previous is None else previous
    result["ok"] = result["error"] is None
    return result

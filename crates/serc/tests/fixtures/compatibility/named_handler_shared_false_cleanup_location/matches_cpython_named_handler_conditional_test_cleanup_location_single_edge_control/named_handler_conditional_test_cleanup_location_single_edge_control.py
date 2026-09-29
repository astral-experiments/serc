def query(flag, fallback):
    try:
        return request()
    except TimeoutError as exc:
        record("timeout")
        if flag:
            raise RuntimeError("request timed out") from exc
    except OSError as exc:
        record("error", str(exc))
        if flag:
            raise RuntimeError(str(exc)) from exc
    return None

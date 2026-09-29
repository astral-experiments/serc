def query(flag, fallback):
    try:
        return request()
    except TimeoutError as exc:
        record("timeout")
        if flag if flag is not None else fallback:
            raise RuntimeError("request timed out") from exc
    except OSError as exc:
        record("error", str(exc))
        if flag if flag is not None else fallback:
            raise RuntimeError(str(exc)) from exc
    return None

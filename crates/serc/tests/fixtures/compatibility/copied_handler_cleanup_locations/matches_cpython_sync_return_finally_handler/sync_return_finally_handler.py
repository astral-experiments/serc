def f(payload, cleanup, record):
    try:
        return payload
    finally:
        try:
            cleanup()
        except Exception as exc:
            if payload is not None:
                record(payload, exc)

def drain(manager, read, expired, idle, ready, report, close):
    stream = manager()
    try:
        while True:
            value = read()
            if expired(value):
                report(value)
                break
            if idle(value):
                report(value)
                break
            report(value)
            if ready(value):
                while True:
                    tail = read()
                    if not tail:
                        break
                    report(tail)
                return value
    finally:
        try:
            close(stream)
        except Exception:
            pass
    return None

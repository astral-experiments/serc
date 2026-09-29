def run(work, recover, cleanup, finish):
    try:
        work()
    except Exception:
        try:
            recover()
        finally:
            cleanup()
        finish()
    return None

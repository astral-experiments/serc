def target(work, cleanup, after, finish):
    try:
        try:
            work()
        finally:
            try:
                cleanup()
            except Exception:
                pass
        after()
    except Exception:
        pass
    finally:
        finish()

def control(work, cleanup, after):
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

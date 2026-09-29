def run(outer, inner, cleanup):
    with outer:
        try:
            with inner:
                return 7
        except Exception:
            raise
        finally:
            if cleanup is not None:
                cleanup.close()

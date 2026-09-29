def f(manager, other):
    try:
        with manager, other:
            pass; pass
            global marker
            try:
                work()
            except Exception:
                recover()
            finally:
                cleanup()
    finally:
        finish()

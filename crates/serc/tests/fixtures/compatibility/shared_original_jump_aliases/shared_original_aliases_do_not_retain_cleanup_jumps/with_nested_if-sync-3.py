def sample(a, b, manager, call, finish):
    while a:
        try:
            if a:
                if b:
                    if a is not None:
                        with manager:
                            call()
        except Exception:
            call()
        finally:
            finish()

def sample(a, b, manager, call, finish):
    while a:
        try:
            if a:
                if b:
                    with manager:
                        call()
        except Exception:
            call()
        finally:
            finish()

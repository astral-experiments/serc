def sample(a, b, call, finish):
    try:
        if a:
            if b:
                if a is not None:
                    try:
                        call()
                    except Exception:
                        call()
    except BaseException:
        call()
    finally:
        finish()

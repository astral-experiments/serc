def sample(a, b, call, finish):
    try:
        if a:
            if b:
                try:
                    call()
                except Exception:
                    call()
    except BaseException:
        call()
    finally:
        finish()

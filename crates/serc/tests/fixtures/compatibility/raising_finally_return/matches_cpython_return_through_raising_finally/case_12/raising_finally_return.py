def f():
    try:
        return 1
    finally:
        try:
            record()
        finally:
            raise RuntimeError()

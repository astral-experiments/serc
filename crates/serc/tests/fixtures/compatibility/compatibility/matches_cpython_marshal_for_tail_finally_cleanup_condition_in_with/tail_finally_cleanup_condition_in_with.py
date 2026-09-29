def f(cm, cw, cond):
    with cm() as tmp:
        with cw():
            for i in range(1, 3):
                try:
                    body(tmp)
                except Exception as e:
                    msg = f"failed: {e}"
                    raise AssertionError(msg)
                finally:
                    if cond:
                        cleanup()

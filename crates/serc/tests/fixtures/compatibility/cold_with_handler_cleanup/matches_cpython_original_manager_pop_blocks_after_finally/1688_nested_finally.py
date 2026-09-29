def f(manager, first, second):
    with manager:
        try:
            prepare()
            with first, second:
                yield
        finally:
            try:
                flush()
                flush_other()
            except Error:
                pass
            try:
                restore()
            finally:
                close()

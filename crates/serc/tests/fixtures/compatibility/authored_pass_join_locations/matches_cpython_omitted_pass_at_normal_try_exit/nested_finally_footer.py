def f(item):
    try:
        try:
            work(item)
            pass
        except Error as exc:
            recover(exc)
        observe(item)
    finally:
        finish(item)
    consume(item)

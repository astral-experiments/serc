def f(item):
    try:
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)

def f(item):
    try:
        work(item)
        42
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)

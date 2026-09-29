def f(item):
    try:
        work(item)
        pass
    except Error:
        recover(item)
    observe(item)
    consume(item)
    finish(item)

def f(items, ready, recover, after):
    try:
        for item in items:
            if ready(item):
                break
    except Error:
        recover()
    after()

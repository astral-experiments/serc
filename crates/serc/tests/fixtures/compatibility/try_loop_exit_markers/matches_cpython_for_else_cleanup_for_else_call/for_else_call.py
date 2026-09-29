def f(items, ready, report, recover, after):
    try:
        for item in items:
            if ready(item):
                break
        else:
            report()
    except Error:
        recover()
    after()

def f(items):
    for item in items:
        if ready(item):
            work(item)
        else:
            finish(item)

def f(flag, items):
    if flag:
        if other():
            work()
        else:
            values = list(items)
            while values:
                consume(values.pop())
            finish()
    else:
        other()
    after()

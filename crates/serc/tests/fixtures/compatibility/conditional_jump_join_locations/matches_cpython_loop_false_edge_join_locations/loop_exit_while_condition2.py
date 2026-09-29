def f(flag, items):
    if flag:
        if other():
            work()
        else:
            values = list(items)
            while not flag:
                consume(values.pop())
    else:
        other()
    after()

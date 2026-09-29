def f(manager, ready, work, after):
    with manager:
        while ready():
            work()
    after()

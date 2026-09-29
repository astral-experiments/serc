def f(ready):
    while ready():
        try:
            try:
                work()
            except ValueError:
                continue
        except RuntimeError:
            recover()

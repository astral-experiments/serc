def f(flag, items):
    if flag:
        for item in items:
            if item:
                break
        else:
            if flag:
                value = 1
                return value
            finish()
    else:
        other()
    after()

def f(flag, values, manager, other):
    try:
        work()
    except Error:
        report()
        if flag:
            try:
                value = primary()
            except Other:
                value = fallback()
            consume(value)
    finish()

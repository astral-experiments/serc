def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            try:
                value = primary()
            except Other:
                value = fallback()
            consume(value)
    finish()

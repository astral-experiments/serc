def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            with manager:
                values = prepare()
            for value in values:
                consume(value)
    finish()

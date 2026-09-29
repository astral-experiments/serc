def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            for value in values:
                consume(value)
    finish()

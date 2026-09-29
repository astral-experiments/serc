def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag:
            with manager:
                prepare()
            consume()
    finish()

def f(work, value, done):
    try:
        work()
    finally:
        match value:
            case 1:
                done(1)
            case 2:
                done(2)
            case _:
                done(0)

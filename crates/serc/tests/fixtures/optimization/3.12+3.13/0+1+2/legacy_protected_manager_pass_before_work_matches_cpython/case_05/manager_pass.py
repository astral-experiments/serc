def f(manager, flag, items):
    try:
        with manager, manager:
            pass
            match flag:
                case 1:
                    work()
                case _:
                    other()
    except Exception:
        recover()

def f(value, work):
    try:
        match value:
            case 1:
                work()
            case 2:
                work()
    except Exception:
        work()
    work()
    return value

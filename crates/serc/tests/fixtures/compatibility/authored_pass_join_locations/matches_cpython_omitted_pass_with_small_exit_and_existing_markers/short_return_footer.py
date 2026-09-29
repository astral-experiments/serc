def f(item):
    try:
        work(item)
        pass
    except Error as exc:
        recover(exc)
    return None

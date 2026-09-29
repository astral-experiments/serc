def f(manager):
    try:
        with manager:
            work()
            'comment'
    except Exception:
        recover()

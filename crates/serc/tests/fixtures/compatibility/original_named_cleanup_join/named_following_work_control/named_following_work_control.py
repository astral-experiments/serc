def f(manager):
    try:
        work()
    except Exception as exc:
        with manager:
            fallback(exc)
    finish()

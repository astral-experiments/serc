def run(work, manager, cached, finish):
    try:
        work()
    except Exception:
        with manager:
            return cached
        finish()
    return None

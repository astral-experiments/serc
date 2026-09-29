def run(work, manager, cached, finish):
    try:
        work()
    except Exception as error:
        with manager:
            return cached
    finish()

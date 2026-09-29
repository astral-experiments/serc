def f(manager, item, flag):
    try:
        with manager:
            if flag is None:
                return 'missing', 'details'
            work()
        return 'ok', None
    except Exception as error:
        return 'error', str(error)
    finally:
        if item is not None and item.ready():
            item.close()

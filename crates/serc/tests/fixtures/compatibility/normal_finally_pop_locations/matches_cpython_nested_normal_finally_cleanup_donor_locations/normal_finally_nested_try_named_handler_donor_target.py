def exercise(items, work, recover, stop, cleanup):
    result = None
    for item in items:
        try:
            result = work(item)
            break
        except RuntimeError as error:
            if stop(item):
                break
            result = recover(error)
        finally:
            try:
                cleanup()
            except ValueError:
                pass
    return result

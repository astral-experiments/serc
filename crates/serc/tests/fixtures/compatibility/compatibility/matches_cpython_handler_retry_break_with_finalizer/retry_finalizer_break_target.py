def run(items, manager, value):
    try:
        work(value)
    except OuterError:
        error = None
        for item in items:
            prepare(item)
            try:
                retry(item, value)
                error = None
                break
            except InnerError as caught:
                error = caught
            finally:
                manager.finished(item)
        if error is not None:
            raise error
    consume(value)

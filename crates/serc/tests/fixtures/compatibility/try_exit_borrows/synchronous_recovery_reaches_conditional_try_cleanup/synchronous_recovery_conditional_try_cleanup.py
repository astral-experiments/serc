def f(sink, item, attempt, limit):
    try:
        work(item)
    except Error as error:
        if attempt >= limit or not recoverable(error):
            raise
        retry(item)
    finally:
        if sink is not None:
            try:
                sink.close()
            except Error:
                pass

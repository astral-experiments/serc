def f(ignore_errors, verbose, data):
    try:
        work()
    except Exception as error:
        message = 'failed'
        if not ignore_errors:
            raise ValueError(message)
        elif verbose:
            report(message)
    return data

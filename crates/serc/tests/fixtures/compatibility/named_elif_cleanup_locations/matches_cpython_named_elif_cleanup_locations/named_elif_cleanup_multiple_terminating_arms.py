def f(strict, returning, silent, call, after):
    try:
        call()
    except Exception as error:
        if strict:
            raise
        elif returning:
            return error
        elif not silent:
            call(error)
    after()

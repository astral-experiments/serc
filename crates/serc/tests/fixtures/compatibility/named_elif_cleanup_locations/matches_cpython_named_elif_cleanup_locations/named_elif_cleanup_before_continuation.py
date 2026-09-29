def f(strict, silent, call, after):
    try:
        call()
    except Exception as error:
        if strict:
            raise
        elif not silent:
            call(error)
    after()

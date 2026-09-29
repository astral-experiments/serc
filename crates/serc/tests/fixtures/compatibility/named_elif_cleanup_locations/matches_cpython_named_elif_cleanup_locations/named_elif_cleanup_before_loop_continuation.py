def f(items, strict, silent, call, after):
    for item in items:
        try:
            call(item)
        except Exception as error:
            if strict:
                raise
            elif not silent:
                call(error)
        after(item)

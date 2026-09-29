def f(items, strict, silent, call):
    for item in items:
        try:
            call(item)
        except Exception as error:
            if strict:
                raise
            elif not silent:
                call(error)

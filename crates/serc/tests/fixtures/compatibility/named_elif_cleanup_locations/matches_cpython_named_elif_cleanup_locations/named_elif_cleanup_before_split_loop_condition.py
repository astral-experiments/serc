def f(items, strict, silent, progress, interval, call, after):
    for item in items:
        try:
            call(item)
        except ValueError as error:
            if strict:
                raise
            elif not silent:
                call(error)
        else:
            try:
                call(item)
            except Exception as error:
                if strict:
                    if not silent:
                        call(error)
                    raise
                elif not silent:
                    call(error)
        if progress and item % interval == 0:
            after(item)

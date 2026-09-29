def f(values, flag):
    for value in values:
        if flag:
            prepare(value)
        else:
            fallback(value)
        try:
            if isinstance(value.payload, Kind):
                consume(value)
            else:
                discard(value)
        except Error:
            recover(value)
        after(value)

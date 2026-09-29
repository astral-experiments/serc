def f(values, flag):
    for value in values:
        if flag:
            raise Error(value)
        try:
            if isinstance(value, Kind):
                consume(value)
            else:
                discard(value)
        except Error:
            recover(value)
        after(value)

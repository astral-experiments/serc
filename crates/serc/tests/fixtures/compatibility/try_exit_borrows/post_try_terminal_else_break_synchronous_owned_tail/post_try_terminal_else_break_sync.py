def f(value, emit):
    try:
        while True:
            item = read()
            if keep(item):
                record(item)
            else:
                finish(item)
                break
    except Error as error:
        record(error)
        pass
    emit(value)

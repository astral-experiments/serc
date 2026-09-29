def f(factory, exists, emit, value):
    output = None
    published = False
    try:
        output = factory()
        emit(output, value)
        output.close()
        published = True
    except Error as error:
        if not exists():
            raise
        published = True
    finally:
        if output is not None and not published:
            output.cancel()
    emit(output, value)
    return value, output

async def f(stop, flag, value, emit):
    try:
        while True:
            if stop:
                break
            if flag:
                tick()
            else:
                tock()
                break
    except Error as error:
        record(error)
        pass
    await emit(value)

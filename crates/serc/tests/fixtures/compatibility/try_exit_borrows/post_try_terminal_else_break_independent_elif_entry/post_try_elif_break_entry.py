async def f(flag, stop, value, emit):
    try:
        while True:
            if flag:
                tick()
            elif stop:
                break
            else:
                tock()
                break
    except Error as error:
        record(error)
        pass
    await emit(value)

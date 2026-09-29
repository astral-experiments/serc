async def f(flag, value, emit):
    try:
        while True:
            if flag:
                tick()
                break
            else:
                tock()
    except Error as error:
        record(error)
        pass
    await emit(value)

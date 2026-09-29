async def f(flag):
    try:
        if flag == 1:
            first()
            pass
        elif flag == 2:
            await last()
            pass
    except Error as error:
        recover(error)
    finally:
        cleanup()
    return flag

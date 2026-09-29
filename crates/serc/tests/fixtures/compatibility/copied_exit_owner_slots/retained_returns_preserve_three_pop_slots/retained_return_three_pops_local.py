async def f(session, receiver, flag):
    try:
        try:
            if flag:
                await receiver.fail()
                status = "failed"
                return
        except InnerError:
            recover()
    except OuterError:
        raise
    finally:
        if session is not None:
            await session.close()

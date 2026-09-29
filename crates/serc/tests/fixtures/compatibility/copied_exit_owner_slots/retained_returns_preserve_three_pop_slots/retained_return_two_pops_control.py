async def f(session, receiver, flag):
    def keep_session():
        return session

    try:
        try:
            if flag:
                await receiver.fail()
                status = "failed"
                return
        except InnerError:
            recover()
    finally:
        if session is not None:
            await session.close()

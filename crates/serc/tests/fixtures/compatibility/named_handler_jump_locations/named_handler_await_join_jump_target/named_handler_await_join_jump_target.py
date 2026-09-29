async def run(flag):
    try:
        submit()
    except Busy:
        note()
        await observe()
        try:
            await send()
        except Error as error:
            if expected(error):
                record()
            else:
                report()
                await acknowledge()
        return
    await finish()

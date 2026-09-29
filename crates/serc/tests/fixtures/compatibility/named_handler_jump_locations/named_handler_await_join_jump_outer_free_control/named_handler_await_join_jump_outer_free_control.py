async def run(flag):
    try:
        await send()
    except Error as error:
        if expected(error):
            record()
        else:
            report()
            await acknowledge()
    return

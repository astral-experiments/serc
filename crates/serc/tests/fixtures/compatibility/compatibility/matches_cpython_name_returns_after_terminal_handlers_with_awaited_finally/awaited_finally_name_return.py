async def run(channel, result):
    try:
        try:
            await channel.work()
        except Error as error:
            raise convert(error) from error
        return result
    finally:
        await channel.close()

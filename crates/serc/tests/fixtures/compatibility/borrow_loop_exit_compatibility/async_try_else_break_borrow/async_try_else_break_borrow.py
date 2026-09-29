async def run(client, delays):
    for delay in delays:
        try:
            response = await client.send()
        except Error as error:
            report(error)
            if delay is None:
                break
        else:
            if response.done:
                return
            if response.retry:
                response.check()
            if delay is None:
                break
        assert delay is not None
        await pause(delay)

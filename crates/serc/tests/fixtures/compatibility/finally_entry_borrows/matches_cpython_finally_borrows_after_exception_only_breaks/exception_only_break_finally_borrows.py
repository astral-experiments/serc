async def probe(resource, stream):
    try:
        prepare()
        while True:
            try:
                await stream.read()
            except Exception as error:
                report(error)
                break
            await stream.write()
    except Exception as error:
        report(error)
    finally:
        report("closing")
        await resource.close()

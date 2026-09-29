async def run(resource, value):
    result = None
    try:
        result = await resource.work(value)
    except Error as error:
        message = str(error)
        report(message)
        assert False
    finally:
        await resource.close()
    return convert(result)

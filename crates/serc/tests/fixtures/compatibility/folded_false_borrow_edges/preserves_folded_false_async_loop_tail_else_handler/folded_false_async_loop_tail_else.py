
async def f(stream, worker):
    async for value in stream:
        if False:
            discard(value)
        else:
            try:
                output = worker.encode(value)
                value.complete(output)
            except Exception as error:
                report(error)
    return worker

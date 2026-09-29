async def f(source, manager, sink, spare):
    resource = None
    try:
        resource = source.open()
        yield "ready"
        while True:
            try:
                async def receive():
                    return await resource.__anext__()
                pending = source.start(receive())
                async with manager:
                    item = await pending
            except TimeoutError:
                sink.timeout()
                yield "timeout"
                if pending is not None:
                    await source.cancel(pending)
                return
            except StopAsyncIteration:
                break
            except Exception as error:
                sink.error(error)
                break
            with sink.context():
                for value in await source.unpack(item):
                    sink.record(value)
                    yield value
                    if value.done:
                        return
    except StopAsyncIteration:
        pass
    finally:
        if resource is not None:
            await resource.aclose()
        if spare is not None:
            await source.cancel(spare)
        sink.finish()

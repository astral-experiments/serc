async def factory(enabled, request, service, queue):
    async def generate():
        try:
            message = None
            stream = service.open_stream()
            iterator = stream_iterator(stream)

            async def stopped():
                if not await request.disconnected():
                    return False
                await iterator.close()
                await queue.put(None)
                return True

            async def emit(message, delta):
                message.extend(delta)
                await queue.put(message)

            if enabled:
                try:
                    first = await wait_for(iterator.next(), timeout=LIMIT)
                except StopAsyncIteration as error:
                    raise RuntimeError('empty stream') from error
                except (TimeoutError, ServiceTimeout) as error:
                    raise RuntimeError('initial timeout') from error
            else:
                first = await first_item(iterator, service)

            if await stopped():
                return
            message = make_message(first)
            await emit(message, first)

            while True:
                try:
                    if enabled:
                        delta = await wait_for(iterator.next(), timeout=LIMIT)
                    else:
                        delta = await iterator.next()
                except StopAsyncIteration:
                    break
                except (TimeoutError, ServiceTimeout) as error:
                    if not enabled:
                        raise
                    raise RuntimeError('stream timeout') from error
                if await stopped():
                    return
                await emit(message, delta)
            assert message is not None
            await queue.put(message)
            await queue.put(None)
        except Cancelled:
            report_cancelled()
    return generate

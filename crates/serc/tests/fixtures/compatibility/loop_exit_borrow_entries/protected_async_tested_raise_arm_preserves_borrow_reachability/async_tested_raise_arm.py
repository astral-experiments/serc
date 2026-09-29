async def sample(stream, receiver, is_error, is_message, stop, consume):
    try:
        async for item in stream:
            if is_error(item):
                raise item
            elif is_message(item):
                should_stop = stop(item)
                if should_stop:
                    break
        receiver.first()
        receiver.second()
    finally:
        receiver.finish()

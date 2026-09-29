async def receive(state, channel, epoch, report, stop, consume, close):
    try:
        while state.enabled and state.channel is channel and not channel.closed:
            item = await channel.receive()
            if item.kind in {1, 2, 3}:
                report(item)
                stop()
                break
            if item.kind is None:
                error = channel.error()
                report(error if error is not None else None)
                stop()
                break
            if item.kind == 4:
                await consume(item)
                continue
            report(item)
    except LookupError:
        raise
    except Exception as error:
        report(error)
        stop()
    finally:
        if state.channel is channel:
            await close(channel, epoch if state.enabled else None)

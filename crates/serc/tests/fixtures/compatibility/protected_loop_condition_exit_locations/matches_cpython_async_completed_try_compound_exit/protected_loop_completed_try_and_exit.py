async def run(items, choose, consume):
    for item in items:
        try:
            if choose(item):
                if item.ready:
                    consume(item)
                else:
                    try:
                        value = obtain(item)
                    except Exception:
                        value = None
                    if isinstance(value, str) and value:
                        consume(value)
                        try:
                            await perform(value)
                            if item.done:
                                consume(item)
                        except Exception as error:
                            consume(error)
            elif item.other:
                consume(item)
            else:
                consume(None)
        except Exception as error:
            consume(error)

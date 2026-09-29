async def probe(values, stream, permit):
    for value in values:
        try:
            match value:
                case 0:
                    result = [item async for item in stream]
                case Kind.VALUE if permit:
                    result = [item async for item in stream]
        except Error:
            recover()

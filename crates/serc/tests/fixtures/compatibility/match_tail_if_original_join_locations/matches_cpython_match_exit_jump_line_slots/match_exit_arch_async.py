async def f(subject, first, second, value):
    match subject:
        case Kind.OTHER:
            return None
        case Kind.WANTED:
            if first:
                value = await read()
                if value:
                    return convert(value)
            elif second:
                await work()
                return convert(value)
    return None

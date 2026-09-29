def nested_tuple_return(outer, middle, inner, extra):
    if outer:
        if middle:
            if inner:
                value = "first"
            else:
                value = "second"
        else:
            value = "third"
    else:
        value = "last"
    return value, extra


async def nested_await_tuple_return(channel, outer, extra):
    initial = await channel.read("initial")
    if outer == "first":
        value = "first"
    elif outer == "nested":
        if await channel.read("middle"):
            inner = await channel.read("inner")
            if inner == initial:
                value = "second"
            else:
                value = "third"
        else:
            value = "fourth"
    elif outer == "await":
        value = await channel.read("value")
    else:
        value = "last"
    return value, extra

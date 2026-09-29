def plain(flag):
    while True:
        if flag():
            break
        use("later")


def prior_constant(flag):
    use("before")
    while True:
        if flag():
            break
        use("later")


def prior_none(flag):
    use(None)
    while True:
        if flag():
            break
        use("later")


def multiple_breaks(value):
    while True:
        if value != 0:
            break
        if value == 0:
            break
        value -= 10


def nested(flag):
    while True:
        while True:
            if flag():
                break
            use("inner")
        if flag():
            break
        use("outer")


async def coroutine(flag):
    while True:
        if await flag():
            break
        use("later")


def bounded(flag):
    while flag():
        if flag():
            break
        use("later")


def following(flag):
    while True:
        if flag():
            break
        use("later")
    use("after")

class Wrapper:
    __match_args__ = ('value',)

def singleton(value):
    match value:
        case None:
            return 0
        case True:
            return 1
        case False:
            return 2
        case 0:
            return 3

def nested(value):
    match value:
        case (None, item):
            return item
        case Wrapper(None):
            return 1
        case None | False:
            return 2

def generator(values):
    for value in values:
        match value:
            case None:
                continue
            case _:
                yield value

async def coroutine(value):
    match value:
        case None:
            return 0
        case True:
            return 1

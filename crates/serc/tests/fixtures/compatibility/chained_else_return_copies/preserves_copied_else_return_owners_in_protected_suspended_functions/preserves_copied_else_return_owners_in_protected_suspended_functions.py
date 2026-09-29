def case_60(item, value, bound, values, flag):
    try:
        if (
            0 <
            item < value
        ):
            return item
        else:
            return value
    except Error:
        recover()

async def case_300(item, value, bound, values, flag):
    await ready()
    try:
        if (
            0 <
            item < value
        ):
            return item
        else:
            return value
    except Error:
        recover()

def case_540(item, value, bound, values, flag):
    yield value
    try:
        if (
            0 <
            item < value
        ):
            return item
        else:
            return value
    except Error:
        recover()

async def case_780(item, value, bound, values, flag):
    await ready()
    yield value
    try:
        if (
            0 <
            item < value
        ):
            yield item
            return
        else:
            yield value
            return
    except Error:
        recover()

def match_sync(value):
    match value:
        case 1:
            with manager():
                work()
    work()

async def match_async(value):
    match value:
        case 1:
            async with manager():
                work()
    await work()

def conditional_match(value, ready):
    match value:
        case 1:
            work()
            if ready:
                try:
                    work()
                except Error:
                    recover()
    work()

def conditional_match_loop(values, ready):
    for value in values:
        match value:
            case 1:
                if ready:
                    try:
                        work()
                    except Error:
                        recover()
            case _:
                work()
        work()

def noninterruptible_if_control(value):
    if value == 1:
        with manager():
            work()
    work()

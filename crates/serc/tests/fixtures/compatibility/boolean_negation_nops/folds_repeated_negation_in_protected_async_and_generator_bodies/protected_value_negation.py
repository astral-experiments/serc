def try_return(flag, value, values):
    try:
        return (
            not
            not
            value
        )
    except Error:
        recover()

def handler_assignment(flag, value, values):
    try:
        work()
    except Error as error:
        result = (
            not not value
        )
        consume(result)
        recover(error)

def finally_assignment(flag, value, values):
    try:
        work()
    finally:
        result = (
            not not value
        )
        consume(result)

async def async_manager(flag, value, values):
    async with manager():
        result = (
            not flag and (not not value)
        )
        consume(result)

def generator_loop(flag, value, values):
    if flag:
        yield value
    for item in values:
        return (
            not
            not
            value
        )

async def async_generator_loop(flag, value, values):
    if flag:
        yield value
    async for item in values:
        result = (
            not not value
        )
        consume(result)

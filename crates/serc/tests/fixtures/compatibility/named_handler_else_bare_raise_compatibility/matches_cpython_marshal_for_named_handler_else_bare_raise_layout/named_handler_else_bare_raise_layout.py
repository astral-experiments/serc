def assignment_follower(value):
    try:
        value = action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
        else:
            raise
    return value

def conjunction_follower(value):
    try:
        value = action(value)
    except Error as error:
        if matches(error) and enabled(value):
            value = fallback(error)
        else:
            raise
    return value

def two_assignments(value):
    try:
        value = action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
            recovered = True
        else:
            raise
    return value, recovered

def expression_try_body(value):
    try:
        action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
        else:
            raise
    return value

async def coroutine_handler(value):
    try:
        await action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
        else:
            raise
    return value

class ClassControl:
    try:
        action(value)
    except Error as error:
        if matches(error):
            value = fallback(error)
        else:
            raise

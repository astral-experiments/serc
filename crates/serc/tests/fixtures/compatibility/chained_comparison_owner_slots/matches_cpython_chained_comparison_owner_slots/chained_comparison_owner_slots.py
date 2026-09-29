
async def after_guard(obj, value):
    if obj.skip:
        return False
    return 3 <= value <= 90

async def after_guard_and_call(obj, value):
    if obj.skip:
        return False
    consume(value)
    return 3 <= value <= 90

def retained_setup(value):
    try:
        return 3 <= value <= 90
    except Error:
        return False

def iterator_target(values):
    for value in values:
        try:
            return 3 <= value <= 90
        except Error:
            pass

def try_except():
    try:
        'try'
        body()
    except:
        'except'
        recover()

def except_only():
    try:
        body()
    except:
        'except'

def nested_handlers():
    try:
        try:
            body()
        except ValueError:
            recover()
        except TypeError:
            recover()
    finally:
        cleanup()

def with_body(manager):
    with manager:
        'with'
        body()

def with_only(manager):
    with manager:
        'with'

async def protected_condition():
    await source
    if test:
        await other

try:
    pass
except (ValueError if True else TypeError):
    pass

try:
    pass
except* (ValueError if True else TypeError):
    pass

def protected_conditions(manager, first, second):
    with manager:
        if first:
            body()
        if second:
            body()

def generator_try():
    yield 1
    try:
        pass
    except Exception:
        pass

if condition:
    try:
        with manager:
            body()
    except Error:
        pass

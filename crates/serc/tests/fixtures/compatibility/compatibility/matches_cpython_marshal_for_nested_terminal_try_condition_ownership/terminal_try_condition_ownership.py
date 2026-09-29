def nested_try(a, b):
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

def complex_condition(a, b, c):
    try:
        if a and (b if c else other()):
            action()
    except Exception:
        recover()

def guarded_match(value, flag):
    try:
        match value:
            case 1 if flag and predicate():
                action()
    except Exception:
        recover()

def nested_try_generator(a, b):
    yield 1
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def nested_try_coroutine(a, b):
    await before()
    try:
        if a:
            try:
                b and await action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def nested_try_async_generator(a, b):
    yield 1
    try:
        if a:
            try:
                b and action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def coroutine_nested_finally(a):
    try:
        if a:
            try:
                action()
            finally:
                cleanup()
    except Exception:
        recover()

async def coroutine_nested_else(a):
    try:
        if a:
            try:
                action()
            except ValueError:
                inner()
            else:
                success()
    except Exception:
        recover()

async def await_late_in_bool_condition(a):
    try:
        if a and await predicate():
            try:
                action()
            except ValueError:
                inner()
    except Exception:
        recover()

async def await_before_nested_finally(a):
    await before()
    try:
        if a:
            try:
                action()
            finally:
                cleanup()
    except Exception:
        recover()

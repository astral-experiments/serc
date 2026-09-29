def call_with_else(items, consume):
    try:
        for item in items:
            if item.ready():
                consume(item)
            else:
                consume(None)
    except Exception:
        return False


def name_with_else(items, consume):
    try:
        for item in items:
            if item:
                consume(item)
            else:
                consume(None)
    except Exception:
        return False


def attribute_with_else(items, consume):
    try:
        for item in items:
            if item.ready:
                consume(item)
            else:
                consume(None)
    except Exception:
        return False


def compare_with_else(items, consume):
    try:
        for item in items:
            if item == 1:
                consume(item)
            else:
                consume(None)
    except Exception:
        return False


def call_without_else(items, consume):
    try:
        for item in items:
            if item.ready():
                consume(item)
    except Exception:
        return False


def call_followed_by_statement(items, consume):
    try:
        for item in items:
            if item.ready():
                consume(item)
            else:
                consume(None)
            consume(item)
    except Exception:
        return False


def call_nested_iterator(items, consume):
    try:
        for item in items:
            if item.ready():
                for value in item.values:
                    consume(value)
            else:
                consume(None)
    except Exception:
        return False


async def coroutine_call_with_else(items, consume):
    try:
        for item in items:
            if item.ready():
                consume(item)
            else:
                consume(None)
    except Exception:
        return False


def generator_call_with_else(items, consume):
    try:
        for item in items:
            if item.ready():
                consume(item)
            else:
                consume(None)
        yield None
    except Exception:
        return False


def call_no_exception_region(items, consume):
    for item in items:
        if item.ready():
            consume(item)
        else:
            consume(None)


def call_with_synchronous_manager(items, consume, manager):
    with manager:
        for item in items:
            if item.ready():
                consume(item)
            else:
                consume(None)

def protected_with_continue(manager, items, magic_key):
    values = set()
    with manager:
        for item in items:
            if not item.startswith(magic_key):
                continue
            pieces = [piece.strip() for piece in item.split(':', 1)]
            if len(pieces) == 2:
                values = values.union(pieces[1].upper().split())
    return values


def protected_iterator_return(items):
    try:
        for item in items:
            if item.ready():
                return item.value().lower()
    except Exception:
        pass
    return None


async def protected_async_iterator_return(items):
    try:
        async for item in items:
            return make(item)
    except Exception:
        pass


def inner_try_return(items):
    for item in items:
        try:
            return item.value()
        except Exception:
            pass


def protected_boolean_return(items):
    try:
        for item in items:
            if item.ready():
                return True
    except Exception:
        pass
    return False


def unprotected_continue(items):
    for item in items:
        if not item.ready():
            continue
        consume(item)

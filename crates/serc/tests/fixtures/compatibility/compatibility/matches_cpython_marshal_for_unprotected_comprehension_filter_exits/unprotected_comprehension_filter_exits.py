def unprotected(items):
    return [item for item in items if keep(item)]

def generator_scope(manager, items):
    with manager:
        values = (item for item in items if keep(item))
    return values

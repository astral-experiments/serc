def applied(manager, condition, items):
    with manager:
        result = [item for item in items]
        if condition:
            return result

def discarded(manager, condition, items):
    with manager:
        {item for item in items}
        if condition:
            return

def retained(manager, condition, items):
    with manager:
        result = consume([item for item in items])
        if condition:
            return result

def nested_else(manager, items, shifted):
    with manager:
        for item in items:
            if shifted:
                first(item)
            else:
                if ready(item):
                    second(item)

def flat_else(manager, items, shifted):
    with manager:
        for item in items:
            if shifted:
                first(item)
            else:
                second(item)

def following(manager, items, shifted):
    with manager:
        for item in items:
            if shifted:
                first(item)
            else:
                if ready(item):
                    second(item)
            after(item)

def outside(items, shifted):
    for item in items:
        if shifted:
            first(item)
        else:
            if ready(item):
                second(item)

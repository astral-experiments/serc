def assigned(items):
    target = "outer"
    [target for target in items]
    first(target)
    second(target)

def parameter(target, items):
    [target for target in items]
    first(target)
    second(target)

def with_target(manager, items):
    with manager as target:
        [target for target in items]
        first(target)
        second(target)

def unrelated(items):
    value = "outer"
    [target for target in items]
    first(value)
    second(value)

def rebound(items):
    target = "outer"
    [target for target in items]
    target = "new"
    first(target)
    second(target)

def discarded(items):
    target = "outer"
    {target for target in items}
    first(target)
    second(target)

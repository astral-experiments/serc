def atomic(items):
    for item in items:
        if first(item):
            if accepted(item):
                consume(item)
        elif second(item):
            fallback(item)


def conjunction(items):
    for item in items:
        if first(item):
            if accepted(item) and ready(item):
                consume(item)
        elif second(item):
            fallback(item)


def disjunction(items):
    for item in items:
        if first(item):
            if accepted(item) or ready(item):
                consume(item)
        elif second(item):
            fallback(item)


def direct_loop_control(items):
    for item in items:
        if accepted(item) and ready(item):
            consume(item)


def with_control(manager, items, match, accept, reject):
    with manager:
        for item in items:
            found = match(item)
            if found:
                if accept(found):
                    accept(item)
                else:
                    reject(found)
            else:
                reject(item)


def try_control(items):
    try:
        for item in items:
            while condition(item):
                consume(item)
    except Exception:
        recover()


def while_control(items):
    for item in items:
        while condition(item):
            consume(item)


def assert_control(items):
    for item in items:
        assert condition(item)

def protected_defaults(items):
    try:
        for item in items:
            first = item.first
            second = item.second
            if first is None:
                first = 0
            if second is None:
                second = 1
            consume(first, second)
    except Error:
        recover()


def direct_control(value):
    try:
        if value is None:
            value = 0
        consume(value)
    except Error:
        recover()


def with_control(manager, value):
    try:
        with manager:
            if value is None:
                value = 0
            consume(value)
    except Error:
        recover()

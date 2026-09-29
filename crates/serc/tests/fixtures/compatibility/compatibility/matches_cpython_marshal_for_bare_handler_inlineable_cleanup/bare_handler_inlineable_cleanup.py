def trailing_try(key):
    try:
        value = first[key]
    except KeyError:
        try:
            value = second[key]
        except KeyError:
            raise NameError(key) from None
    return value


def trailing_if(flag):
    try:
        action()
    except Error:
        value = None
        if flag:
            value = recover()
        else:
            value = fallback()
    return value


def trailing_if_with_expression_join(flag):
    try:
        action()
    except Error:
        value = None
        if flag:
            value = (
                first()
                or second()
            )
        else:
            value = fallback()
    return value


def trailing_if_with_terminating_arm(first, second):
    try:
        action()
    except Error:
        if first:
            raise
        elif second:
            value = recover()
        else:
            value = fallback()
    return value


def nested_handler_return_before_following(first, second):
    try:
        value = first()
    except Error:
        try:
            result = second()
        except OtherError:
            return None
        def normalize(value):
            return value
        return normalize(result)
    return value

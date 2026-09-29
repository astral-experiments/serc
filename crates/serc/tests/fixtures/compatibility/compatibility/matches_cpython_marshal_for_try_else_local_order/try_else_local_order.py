def ordered():
    try:
        initial = produce()
    except Exception as error:
        handler_only = fallback()
        result = handler_only
    else:
        result = success(initial)
    return result


def no_else_control():
    try:
        initial = produce()
    except Exception as error:
        handler_only = fallback()
    return initial

def nested_try(first, second):
    try:
        if first:
            if second:
                value = work()
            else:
                value = other()
        else:
            value = fallback()
    except Error:
        value = recover()
    return value

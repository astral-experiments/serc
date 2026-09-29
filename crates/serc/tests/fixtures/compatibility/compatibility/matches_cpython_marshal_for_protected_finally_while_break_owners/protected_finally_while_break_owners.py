def initialized_before_try(frame):
    result = 0
    try:
        while frame:
            if ready(frame):
                break
            step(frame)
    finally:
        del frame
    return result


def initialized_inside_try(frame):
    try:
        result = 0
        while frame:
            if ready(frame):
                break
            step(frame)
    finally:
        del frame
    return result


def generator_for_control(items):
    yield value
    try:
        for item in items:
            if ready(item):
                break
            step(item)
    finally:
        del items
    return value


def handler_control(items):
    for item in items:
        try:
            consume(item)
        except Error:
            break
    return value

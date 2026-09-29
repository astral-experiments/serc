def named_handler(callback, value):
    try:
        return callback()
    except Exception as error:
        return "error=%s" % (error,)


def bare_handler(callback, value):
    try:
        return callback()
    except Exception:
        return "error=%s" % (value,)


def try_else(obj, value):
    try:
        name = obj.name
    except AttributeError:
        return value
    else:
        return "name=%s" % (name,)


def entry_block(value):
    return "entry=%s" % (value,)


def preceded_in_handler(callback, value):
    try:
        return callback()
    except Exception:
        result = value
        return "error=%s" % (result,)


def shared_join(flag, left, right):
    if flag:
        value = left
    else:
        value = right
    return "value=%s" % (value,)

def protected_branch_return(flag):
    try:
        if flag:
            try:
                action()
            except Error:
                raise ValueError
        return value
    except ValueError:
        return fallback


def try_else_nested_join(value, names, other):
    try:
        action()
    except TypeError:
        recover()
    else:
        if names is not None:
            if value.names is None:
                first()
            else:
                second()
        elif other:
            third()
    return value

def before_return(flag):
    try:
        prepare()
        if flag:
            raise ValueError
    except TypeError:
        recover()
    return flag

def before_raise(flag):
    try:
        prepare()
        if flag:
            raise ValueError
    except TypeError:
        recover()
    raise RuntimeError

def before_conditional_return(values, required):
    for value in values:
        result = None
        try:
            result = prepare(value)
            if required and value is None:
                raise ValueError(value)
        except TypeError:
            recover(value)
        except LookupError:
            recover_again(value)
        if result is not None:
            return value
    return None

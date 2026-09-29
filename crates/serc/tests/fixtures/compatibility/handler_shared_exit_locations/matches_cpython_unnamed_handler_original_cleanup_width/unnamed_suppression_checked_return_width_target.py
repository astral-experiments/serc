def recover(flag, manager):
    try:
        result = work()
    except LookupError:
        if flag:
            with manager:
                raise ValueError()
        else:
            raise TypeError()
    return result

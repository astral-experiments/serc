def recover(flag, manager):
    try:
        result = work()
    except LookupError:
        if flag:
            with manager:
                result = repair()
        else:
            raise TypeError()
    return result

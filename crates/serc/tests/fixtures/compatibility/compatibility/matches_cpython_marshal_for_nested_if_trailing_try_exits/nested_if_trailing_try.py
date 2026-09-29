def nested_try(flag, outer):
    if outer:
        if flag:
            try:
                action()
            except Error:
                raise
    else:
        other()
    return None

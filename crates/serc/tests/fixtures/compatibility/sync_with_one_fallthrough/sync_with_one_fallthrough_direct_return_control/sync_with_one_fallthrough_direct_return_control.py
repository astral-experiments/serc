def direct_return(manager, condition, value, report):
    with manager:
        if condition:
            return value
        else:
            report(value)
    return None

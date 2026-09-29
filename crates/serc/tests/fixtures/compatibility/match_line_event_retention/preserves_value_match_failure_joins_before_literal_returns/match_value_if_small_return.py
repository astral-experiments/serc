def f(flag, value, work):
    if flag:
        match value:
            case "a":
                return work(value)
            case "b":
                return work(value)
    elif value:
        return work(flag)
    return None

def f(flag, value, work):
    if flag:
        match value:
            case Kinds.first:
                return work(value)
            case Kinds.second if work(flag):
                return work(value)
    elif value:
        return work(flag)
    return None

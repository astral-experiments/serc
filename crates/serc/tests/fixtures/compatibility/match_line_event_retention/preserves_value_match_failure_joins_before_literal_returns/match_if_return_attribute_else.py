def f(flag, value, work):
    if flag:
        match value:
            case Kinds.first:
                return work(value)
            case Kinds.second:
                return work(value)
    else:
        return work(flag)
    return None

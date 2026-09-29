def f(value, work):
    match value:
        case Outer(item=item):
            match item:
                case Inner(data=data):
                    return data
            work(item)
    return None

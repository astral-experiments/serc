def f(value):
    match value:
        case Outer(item=item):
            match item:
                case Inner(data=data):
                    return data
    return None

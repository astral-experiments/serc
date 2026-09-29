def f(value, work):
    match value:
        case Outer(item=item):
            match item:
                case Inner():
                    work(item)
                case _:
                    work(item)
    return None

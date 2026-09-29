def f(value, work):
    match value:
        case Outer(item=item):
            match item:
                case Inner():
                    if work(item):
                        return False
                case None:
                    pass
                case _:
                    work(item)
        case _:
            raise ValueError(value)
    return True

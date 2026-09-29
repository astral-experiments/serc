def f(value, work, values):
    events = [work(element) for element in values]
    match value:
        case Outer(item=item):
            match item:
                case Inner():
                    if work(item):
                        work(value)
                case _:
                    work(item)
        case _:
            raise ValueError(value)
    return events

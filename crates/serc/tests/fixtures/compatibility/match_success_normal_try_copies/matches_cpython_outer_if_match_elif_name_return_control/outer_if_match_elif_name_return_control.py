def f(mode, active, first, second, work, items):
    if active:
        match mode:
            case Mode.FIRST:
                work(items)
            case Mode.LAST:
                if first:
                    work(items)
                elif second:
                    work(items)
    return items

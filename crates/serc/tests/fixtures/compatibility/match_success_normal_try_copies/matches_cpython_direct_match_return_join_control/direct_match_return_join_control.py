def f(items, mode, active, ready, report):
    match mode:
        case Mode.FIRST:
            report(items)
            if ready:
                return [items[0]]
        case Mode.SEARCH:
            found = None
            for item in items:
                if item.ready:
                    found = item
                    break
            if found is not None:
                return [found]
            else:
                report(items)
        case Mode.ALL:
            report(items)
            pass
        case _:
            raise ValueError(mode)
    return items

def probe(items, first, second, emit):
    for item in items:
        if item.active:
            if first(item) and second(item):
                emit(item)
        elif item.stop:
            break
    else:
        return None
    return True

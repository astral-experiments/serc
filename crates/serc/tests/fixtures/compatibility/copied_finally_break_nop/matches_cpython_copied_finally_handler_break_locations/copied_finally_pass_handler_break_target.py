def choose(paths, ready, remove):
    index = 0
    result = None
    while index < len(paths):
        current = paths[index]
        try:
            result = ready(current)
            if result is not None:
                break
        finally:
            if current is not None:
                try:
                    remove(current)
                except FileNotFoundError:
                    pass
        index += 1
    if result is None:
        raise ValueError("no result")
    return result

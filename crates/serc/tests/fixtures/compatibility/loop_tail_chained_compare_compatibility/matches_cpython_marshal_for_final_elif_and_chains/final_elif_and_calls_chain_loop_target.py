def restore(items):
    for parent, key, value in items:
        if mapping(parent):
            parent[key] = value
        elif (
            sequence(parent)
            and integer(key)
            and 0 <= key < len(parent)
        ):
            parent[key] = value

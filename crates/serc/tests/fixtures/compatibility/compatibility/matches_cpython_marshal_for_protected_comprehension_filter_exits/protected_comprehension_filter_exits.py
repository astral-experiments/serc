def with_owner(manager, items):
    with manager:
        values = [item.name for item in items if item.is_file()]
    return values

def none_filter_owner(manager, items):
    with manager:
        present = [item for item in items if item is not None]
    return present

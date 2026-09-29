
def with_list_clean(manager, outer, values):
    with manager:
        return [tuple(x for x in values) for y in outer]

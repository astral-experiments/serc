
class Error(Exception):
    pass


def different_object_control(items, other):
    for item in items:
        try:
            use(item.name if other.name else "fallback")
        except Error:
            continue

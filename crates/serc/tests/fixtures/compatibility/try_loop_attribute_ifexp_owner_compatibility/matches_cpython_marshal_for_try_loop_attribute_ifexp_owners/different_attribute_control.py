
class Error(Exception):
    pass


def different_attribute_control(items):
    for item in items:
        try:
            use(item.name if item.title else "fallback")
        except Error:
            continue

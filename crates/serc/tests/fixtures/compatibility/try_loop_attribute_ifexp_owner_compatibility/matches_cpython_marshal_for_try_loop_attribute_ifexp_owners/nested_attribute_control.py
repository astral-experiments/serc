
class Error(Exception):
    pass


def nested_attribute_control(items):
    for item in items:
        try:
            use(item.meta.name if item.meta.name else "fallback")
        except Error:
            continue

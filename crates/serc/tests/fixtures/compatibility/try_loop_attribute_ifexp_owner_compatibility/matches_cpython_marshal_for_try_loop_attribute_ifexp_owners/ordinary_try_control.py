
class Error(Exception):
    pass


def ordinary_try_control(item):
    try:
        return item.name if item.name else "fallback"
    except Error:
        return None

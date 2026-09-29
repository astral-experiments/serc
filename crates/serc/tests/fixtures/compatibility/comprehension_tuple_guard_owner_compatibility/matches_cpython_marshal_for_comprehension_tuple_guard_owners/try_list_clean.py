
class Error(Exception):
    pass


def try_list_clean(outer, values):
    try:
        return [tuple(x for x in values) for y in outer]
    except Error:
        return None

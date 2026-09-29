
class Error(Exception):
    pass


def concrete_pair_control(items, left, right):
    for item in items:
        try:
            pair = (left, right)
            use(item.name if item.name else "fallback")
        except Error:
            return pair

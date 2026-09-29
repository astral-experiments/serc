def tuple_return(left, right, limit):
    if left == right == limit:
        return left, left
    consume()
    return fallback()


def direct_raise(left, right, limit):
    if left == right == limit:
        raise ValueError
    consume()
    return fallback()


def multi_statement_control(left, right, limit):
    if left == right == limit:
        consume(left)
        return left, left
    consume()
    return fallback()


def final_none_control(first, second):
    if first is second is None:
        return (None,) * 4
    consume()
    return fallback()

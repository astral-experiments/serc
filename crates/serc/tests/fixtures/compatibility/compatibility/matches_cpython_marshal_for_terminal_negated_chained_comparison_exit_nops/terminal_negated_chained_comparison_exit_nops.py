def direct_raise(low, value, high):
    if not (low <= value < high):
        raise IndexError
    consume()


def prefixed_elif(kind, enabled, low, value, high):
    if kind == "simple":
        consume()
    elif enabled and not (low <= value < high):
        raise IndexError
    finish()


def direct_return(low, value, high):
    if not (low <= value < high):
        return value
    consume()


def two_negated_chains(a, b, c, d, e, f):
    if not (a < b < c) and not (d < e < f):
        raise ValueError
    finish()


def terminating_else(low, value, high):
    if not (low <= value < high):
        raise IndexError
    else:
        raise ValueError


def loop_raise_after_try(items, enabled, low, value, high):
    for item in items:
        try:
            prepare(item)
        except ValueError:
            recover()
        if item is None:
            consume(item)
        elif enabled and not (low <= value < high):
            raise IndexError
        finish(item)


def final_operand_control(low, value, high, enabled):
    if not (low <= value < high) and enabled:
        raise IndexError
    consume()


def multi_statement_control(low, value, high):
    if not (low <= value < high):
        consume()
        raise IndexError
    finish()


def loop_controls(items, low, value, high):
    for item in items:
        if not (low <= value < high):
            break
        consume(item)
    for item in items:
        if not (low <= value < high):
            continue
        consume(item)


def loop_return(items, low, high):
    for item in items:
        if not (low <= item <= high):
            return False
    return True

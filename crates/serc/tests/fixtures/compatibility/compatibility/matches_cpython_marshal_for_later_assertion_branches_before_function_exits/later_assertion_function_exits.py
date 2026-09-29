def before_return(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    return


def before_raise(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    raise RuntimeError


def ordinary_successor_control(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        consume()
    after()


def terminating_else_control(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        return
    after()


def terminating_else_before_return(first, second):
    if first:
        consume()
    elif second:
        assert second.value
    else:
        return
    return


def loop_controls(items, first, second):
    for item in items:
        if first:
            consume()
        elif second:
            assert second.value
        else:
            consume()
        break
    for item in items:
        if first:
            consume()
        elif second:
            assert second.value
        else:
            consume()
        continue

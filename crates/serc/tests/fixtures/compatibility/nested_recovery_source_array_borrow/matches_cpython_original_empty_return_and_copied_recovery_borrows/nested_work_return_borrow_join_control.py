def f(node, outer, middle, choice, inner, value):
    if outer:
        value = node.start(value)
        if middle:
            value = node.prepare(value)
            if choice:
                value = node.next(value)
                if inner:
                    value = node.finish(value)
                else:
                    value = node.recover(value)
            else:
                value = node.other(value)
    else:
        value = node.fallback(value)
    return value

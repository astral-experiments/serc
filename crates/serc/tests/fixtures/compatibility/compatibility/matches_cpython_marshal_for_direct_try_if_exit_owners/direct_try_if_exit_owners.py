def joined_loop(items, first, second):
    for item in items:
        if first:
            prefix = item
        else:
            prefix = None
        try:
            if second:
                value = item
            else:
                value = prefix
        except ValueError:
            value = None
        consume(value)


def nested_first_branch(items, outer, inner):
    for item in items:
        try:
            if outer:
                if inner:
                    value = item
                else:
                    value = None
            else:
                value = 0
        except ValueError:
            value = -1
        consume(value)


def nested_call_branch(first, second, rowtype, coltype, item):
    try:
        if first:
            row = rowtype(item)
        else:
            row = coltype(item)
        if second:
            result = row.dtype.char
        else:
            value = row + item
            if predicate(value):
                result = value.dtype.char
            else:
                result = type(value).dtype.char
    except ValueError:
        result = '!'
    except TypeError:
        result = '#'
    consume(result)


def try_else_control(items, flag):
    for item in items:
        try:
            if flag:
                value = item
            else:
                value = None
        except ValueError:
            value = 0
        else:
            consume(value)

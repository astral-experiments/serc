def choose(outer, first, second, other):
    if outer:
        if first:
            if second:
                report(second)
                result = 1
            else:
                result = 2
        else:
            result = 3
    else:
        result = 4
    return result, other

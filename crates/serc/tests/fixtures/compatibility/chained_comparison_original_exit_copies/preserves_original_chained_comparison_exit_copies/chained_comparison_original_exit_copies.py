def identity_none__local(a, b, c, d, result, flag):
    if a is b is None:
        return result
    consume(result)
    return d


def identity_none__attribute(a, b, c, d, result, flag):
    if a is b is None:
        return result.value
    consume(result)
    return d


def identity_none__call(a, b, c, d, result, flag):
    if a is b is None:
        return result()
    consume(result)
    return d


def identity_none__big_call(a, b, c, d, result, flag):
    if a is b is None:
        return result(a, b, c)
    consume(result)
    return d


def identity_none__tuple(a, b, c, d, result, flag):
    if a is b is None:
        return a, b, c
    consume(result)
    return d


def identity_none__fallthrough(a, b, c, d, result, flag):
    if a is b is None:
        result = a
    consume(result)
    return d


def identity_none__raise(a, b, c, d, result, flag):
    if a is b is None:
        raise Error
    consume(result)
    return d


def identity_none__raise_call(a, b, c, d, result, flag):
    if a is b is None:
        raise Error(a, b)
    consume(result)
    return d


def and_identity__local(a, b, c, d, result, flag):
    if flag and a is b is None:
        return result
    consume(result)
    return d


def or_identity__attribute(a, b, c, d, result, flag):
    if flag or a is b is None:
        return result.value
    consume(result)
    return d


def negated_identity__local(a, b, c, d, result, flag):
    if not (a is b is None):
        return result
    consume(result)
    return d


def terminal_pass_else(a,b,c,d):
    if a == b == c:
        pass
    else:
        consume(d)


def elif_pass(a,b,c,d):
    if a in b is c in d:
        pass
    elif a not in b is d in c:
        pass


def while_break_copy_112(a,b,c,result):
    while c:
        consume(result)
        if 0 < a < b:
            break
    return result


def while_break_copy_113(a,b,c,result):
    while c:
        consume(result)
        if 0 < a < b:
            break
    return result.value


def while_break_copy_114(a,b,c,result):
    while c:
        consume(result)
        if 0 < a < b:
            break
    return result(a,b,c)


def while_break_copy_115(a,b,c,result):
    while c:
        consume(result)
        if 0 < a < b:
            break
    consume(result)
    return result


async def while_break_copy_116(a,b,c,result):
    while c:
        await consume(result)
        if 0 < a < b:
            break
    return result


async def while_break_copy_118(a,b,c,result):
    while c:
        await consume(result)
        if 0 < a < b:
            break
    return result(a,b,c)


def multiline_identity(a,b,c,d):
    if (a
        is b
        is None):
        return c
    consume(d)


def folded_identity_none__none(a,b,c):
    if a is b is None:
        return None
    consume()
    return c


def folded_identity_none__tuple_repeat(a,b,c):
    if a is b is None:
        return (None,) * 4
    consume()
    return c


def folded_identity_none__tuple(a,b,c):
    if a is b is None:
        return (None, None, None, None)
    consume()
    return c


def folded_identity_none__folded_binary(a,b,c):
    if a is b is None:
        return 2+3
    consume()
    return c

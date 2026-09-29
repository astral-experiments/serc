def assignment_chain(kind, value, consume):
    try:
        if kind == "one":
            result = value.one()
        elif kind == "two":
            result = value.two()
        elif kind == "three":
            result = value.three()
        else:
            raise ValueError(kind)
    except ValueError:
        return None
    return consume(result)


def expression_chain(kind, value, consume):
    try:
        if kind == "one":
            value.one()
        elif kind == "two":
            value.two()
        elif kind == "three":
            value.three()
        else:
            raise ValueError(kind)
    except ValueError:
        return None
    return consume(kind)


def mixed_chain(kind, value, consume):
    result = None
    try:
        if kind == "one":
            value.one()
        elif kind == "two":
            result = value.two()
        elif kind == "three":
            result += value.three()
        else:
            raise ValueError(kind)
    except ValueError as error:
        return consume(error)
    return consume(result)


def nonterminating_fallback(kind, value, consume):
    try:
        if kind == "one":
            result = value.one()
        elif kind == "two":
            result = value.two()
        else:
            result = value.three()
    except ValueError:
        return None
    return consume(result)


def following_statement_control(kind, value, consume):
    try:
        if kind == "one":
            result = value.one()
        elif kind == "two":
            result = value.two()
        elif kind == "three":
            result = value.three()
        else:
            raise ValueError(kind)
        consume(kind)
    except ValueError:
        return None
    return consume(result)


def try_else_control(kind, value, consume):
    try:
        if kind == "one":
            result = value.one()
        elif kind == "two":
            result = value.two()
        elif kind == "three":
            result = value.three()
        else:
            raise ValueError(kind)
    except ValueError:
        return None
    else:
        return result


def finally_control(kind, value, cleanup):
    try:
        if kind == "one":
            result = value.one()
        elif kind == "two":
            result = value.two()
        elif kind == "three":
            result = value.three()
        else:
            raise ValueError(kind)
    finally:
        cleanup()
    return result


async def coroutine_control(kind, value):
    try:
        if kind == "one":
            result = await value.one()
        elif kind == "two":
            result = await value.two()
        elif kind == "three":
            result = await value.three()
        else:
            raise ValueError(kind)
    except ValueError:
        return None
    return result

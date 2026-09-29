def trailing_fallthrough_try(key):
    try:
        value = primary[key]
    except KeyError:
        if key:
            value = fallback[key]
        try:
            value = secondary[key]
        except KeyError:
            report(key)
    return value


def following_handler_statement_control(key):
    try:
        value = primary[key]
    except KeyError:
        try:
            value = secondary[key]
        except KeyError:
            report(key)
        consume(value)
    return value


def nested_handler_branch_control(key):
    try:
        value = primary[key]
    except KeyError:
        if key:
            try:
                value = secondary[key]
            except KeyError:
                report(key)
        consume(value)
    return value

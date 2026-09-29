def nested_branch(flag, header):
    try:
        value = parse(header)
    except SyntaxError as error:
        if flag:
            try:
                value = parse_legacy(header)
            except SyntaxError as nested:
                raise ValueError(header) from nested
            else:
                warn(header)
        else:
            raise ValueError(header) from error
    consume(value)
    return value


def direct_control(header):
    try:
        value = parse(header)
    except SyntaxError as error:
        try:
            value = parse_legacy(header)
        except SyntaxError as nested:
            raise ValueError(header) from nested
        else:
            warn(header)
    consume(value)
    return value

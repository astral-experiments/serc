def process_file(source, value):
    try:
        code = convert(value)
    except ValueError as error:
        raise ValueError('In "%s" loop at %s' % (source, error)) from None
    return '#line 1 "%s"\n%s' % (source, code)


def return_from_handler(value):
    try:
        converted = int(value)
    except ValueError:
        return value
    return "converted=%s" % (converted,)


def conditional_handler(value, recover):
    try:
        converted = int(value)
    except ValueError:
        if recover:
            converted = value
        else:
            raise
    return "converted=%s" % (converted,)

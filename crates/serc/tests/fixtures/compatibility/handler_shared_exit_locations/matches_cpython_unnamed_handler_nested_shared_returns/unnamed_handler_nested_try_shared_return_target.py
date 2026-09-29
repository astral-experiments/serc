def select_value(value, first):
    result = {}
    try:
        check(value)
    except ValueError:
        if first:
            try:
                result["value"] = convert(value)
            except TypeError:
                result["value"] = None
        else:
            result["value"] = value
    return result

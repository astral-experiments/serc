def recovered_value(value):
    try:
        converted = int(value)
    except ValueError:
        converted = value
    return "(%s %s %%s)" % (value, value), converted

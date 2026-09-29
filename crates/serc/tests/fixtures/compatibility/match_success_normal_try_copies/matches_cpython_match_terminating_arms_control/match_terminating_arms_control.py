def choose(value):
    try:
        match value:
            case 'first':
                return first(value)
            case 'second':
                return second(value)
    except Error:
        return recover(value)
    raise Failure(value)

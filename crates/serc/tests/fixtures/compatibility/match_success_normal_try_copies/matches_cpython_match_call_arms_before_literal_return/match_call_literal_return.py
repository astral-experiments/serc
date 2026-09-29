def choose(value):
    try:
        match value:
            case 'first':
                first()
            case 'second':
                second()
    except Error:
        return 1
    return 0

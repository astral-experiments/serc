def choose(options):
    try:
        value, extra = prepare(options)
        match options.mode:
            case 'first':
                first(value, extra)
            case 'second':
                second(value)
            case 'third':
                third(value, extra)
    except Exception as error:
        report(error)
        return 1
    return 0

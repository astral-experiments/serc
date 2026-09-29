def f(value):
    value = value.replace("/", "-")
    if accept(value):
        first, second = value.split()
        if len(first) == 3:
            number = short_names.index(first.lower())
        else:
            number = long_names.index(first.title())
        if check(second) and number:
            value = f"{second}-{number:02d}-01"
    else:
        match len(value):
            case 4:
                value = f"{value}-01-01"
            case 7:
                if check(value[:4]):
                    value = value + "-01"
                elif check(value[-4:]):
                    value = f"{value[-4:]}-{value[0:2]}-01"
            case 8:
                if check(value[-4:]):
                    value = value[0:2] + "-" + value[2:4] + "-" + value[4:]
                elif check(value[:4]):
                    value = value[0:4] + "-" + value[4:6] + "-" + value[6:]
    try:
        parsed = parser.parse(value).isoformat()
        if check(parsed[:4]):
            return parsed
    except Exception as error:
        print(error)
    return ""

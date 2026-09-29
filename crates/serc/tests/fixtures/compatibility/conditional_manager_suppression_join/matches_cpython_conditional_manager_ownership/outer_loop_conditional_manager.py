def convert(managers, inner, choose, value):
    for outer in managers:
        with outer:
            if choose:
                with inner:
                    return value
            else:
                return value

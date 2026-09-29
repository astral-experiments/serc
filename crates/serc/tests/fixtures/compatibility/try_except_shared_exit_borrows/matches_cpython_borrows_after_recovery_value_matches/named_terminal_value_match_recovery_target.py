def edit(value, read, parse, choose, report):
    while True:
        text = read(value)
        if text is None:
            return
        try:
            result = parse(text)
        except ValueError as error:
            report(error)
            action = choose()
            match action:
                case "again":
                    continue
                case "reset":
                    value = text
                    continue
                case "stop":
                    report(error)
                    return
                case _:
                    raise ValueError(action) from None
        formatted = str(result)
        report(formatted, value)
        if choose():
            return formatted

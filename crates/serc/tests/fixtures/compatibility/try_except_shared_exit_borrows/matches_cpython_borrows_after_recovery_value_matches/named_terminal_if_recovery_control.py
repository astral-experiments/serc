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
            if action == "again":
                continue
            elif action == "reset":
                value = text
                continue
            elif action == "stop":
                report(error)
                return
            else:
                raise ValueError(action) from None
        formatted = str(result)
        report(formatted, value)
        if choose():
            return formatted

def collect(rows, parse, output, report, ignore_errors, finish):
    for kind, payload in rows:
        if kind == "record":
            try:
                value = parse(payload)
            except ValueError as error:
                if not ignore_errors:
                    raise error
                report(payload)
            else:
                if value is not None:
                    output.append(value)

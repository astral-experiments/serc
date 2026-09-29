def validate(load, parse, attest, changed, valid, fields):
    errors = []
    try:
        data = load()
    except OSError as error:
        errors.append(str(error))
    else:
        if changed(data):
            errors.append("changed")
        try:
            value = parse(data)
        except ValueError as error:
            errors.append(str(error))
        else:
            if not valid(value):
                errors.append("invalid")
            else:
                for field in fields:
                    if changed(field):
                        errors.append(field)
                try:
                    result = attest(value)
                except RuntimeError as error:
                    errors.append(str(error))
                else:
                    if changed(result):
                        errors.append("attested")
    if errors:
        raise RuntimeError("; ".join(errors))

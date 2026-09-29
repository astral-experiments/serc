def f(data, check, load, report):
    value = None
    try:
        value = load(data)
        report(value)
        if data and check(data):
            report(data)
            while True:
                report(value)
                try:
                    value = load(data)
                except Exception as error:
                    report(error)
                    continue
                report(value)
                if check(value):
                    report(data)
                    break
    except Exception as error:
        report(error)
    result = load(data)
    report(value, result)
    if check(result):
        result = load(value)
    report(data, result)
    return result

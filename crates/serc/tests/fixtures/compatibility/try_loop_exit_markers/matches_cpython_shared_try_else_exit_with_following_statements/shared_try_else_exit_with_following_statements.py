def f(values, flag):
    for value in values:
        if flag:
            if predicate(value):
                try:
                    result = read(value)
                except Error as error:
                    report(error)
                else:
                    expected = transform(value) if predicate(value) else value
                    if result != expected:
                        report(result)
        else:
            if not predicate(value):
                continue
            report(value)
        consume(value)

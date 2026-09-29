def collect(options, value):
    result = {}
    try:
        batches = initial(value)
        for batch in batches:
            merge(result, batch)
    except Error:
        for option in options:
            try:
                batches = retry(option, value)
            except Error:
                batches = ()
            for batch in batches:
                merge(result, batch)
    return [result[name] for name in sorted(result)]

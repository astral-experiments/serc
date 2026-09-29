def read_fallback(open_resource, read, report):
    try:
        with open_resource('primary') as handle:
            result = read(handle)
    except ValueError:
        try:
            with open_resource('fallback') as handle:
                result = read(handle)
        except ValueError:
            report()
            result = []
    return result

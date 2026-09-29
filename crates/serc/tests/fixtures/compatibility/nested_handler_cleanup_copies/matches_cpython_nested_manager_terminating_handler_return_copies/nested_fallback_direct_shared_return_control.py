def read_fallback(open_resource, read, report):
    try:
        with open_resource('primary') as handle:
            result = read(handle)
    except ValueError:
        try:
            result = read(open_resource('fallback'))
        except ValueError:
            report()
            result = []
    return result

def read_cached(kind, open_resource, load, build, save):
    try:
        if kind == 'text':
            with open_resource('r') as handle:
                result = load(handle)
        elif kind == 'bytes':
            with open_resource('rb') as handle:
                result = load(handle)
        else:
            raise ValueError('unknown format')
    except FileNotFoundError:
        result = build()
        save(result)
    return result

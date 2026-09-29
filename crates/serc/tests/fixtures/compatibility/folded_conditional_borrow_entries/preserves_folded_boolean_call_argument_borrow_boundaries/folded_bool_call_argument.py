def f(options, value, consume):
    result = options.get('key' or 'fallback')
    consume(result, value)
    return value

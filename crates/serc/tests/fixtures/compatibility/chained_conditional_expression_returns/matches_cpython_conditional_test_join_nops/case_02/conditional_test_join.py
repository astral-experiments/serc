def f(value, flag):
    if check(value) if flag else False:
        return None
    result = convert(value)
    use(result)
    return str(result)

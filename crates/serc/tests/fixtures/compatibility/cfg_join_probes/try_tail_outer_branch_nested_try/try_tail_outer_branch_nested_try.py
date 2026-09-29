def f(flag, request):
    value = None
    result = {}
    if not flag:
        try:
            response = request.read()
            if response is not None:
                value = response.status
                if value >= 400:
                    result['error'] = 1
                else:
                    try:
                        result = consume(response.read())
                    except InnerError:
                        result = {}
        except OuterError:
            value = None
    if value is not None and value >= 400:
        sink(value)
    sink(result)
    return finish()

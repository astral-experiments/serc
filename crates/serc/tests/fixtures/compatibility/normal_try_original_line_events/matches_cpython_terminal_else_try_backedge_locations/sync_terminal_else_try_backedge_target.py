def f(items, choose, load):
    for item in items:
        response = None
        try:
            mode = choose(item)
            if mode == 1:
                return load(item)
            elif mode == 2:
                return load(item)
            else:
                response = load(item)
                if item not in response.failed:
                    return response
        except Exception as exc:
            record(exc)
    finish()
    return None

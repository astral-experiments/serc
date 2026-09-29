def f(step, done, value):
    try:
        while True:
            step()
            if done():
                break
    except OSError:
        return None
    result: list = []
    for item in value:
        result.append(item)
    return result

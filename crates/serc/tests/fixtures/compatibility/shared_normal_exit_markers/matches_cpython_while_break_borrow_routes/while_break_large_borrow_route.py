def f(step, done, value, consume):
    try:
        while True:
            step()
            if done():
                break
    except OSError:
        return None
    consume(value)
    return value

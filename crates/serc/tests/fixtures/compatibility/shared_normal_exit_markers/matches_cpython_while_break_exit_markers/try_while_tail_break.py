def f(ready, step, result):
    try:
        while True:
            step()
            if ready():
                break
    except RuntimeError:
        return None
    return result

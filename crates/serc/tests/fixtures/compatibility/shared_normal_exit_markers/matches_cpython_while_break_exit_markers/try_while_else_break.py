def f(ready, step, result):
    try:
        while True:
            step()
            if ready():
                result = step()
            else:
                break
    except RuntimeError:
        recover()
    return result

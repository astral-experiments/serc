def f(ready, other, step, finish, result):
    try:
        while True:
            step()
            if ready() or other():
                break
    except RuntimeError:
        try:
            while True:
                step()
                if ready() or other():
                    break
        except ValueError:
            step()
    finish(result)
    return result

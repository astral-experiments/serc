def f(ready, other, step, finish, result):
    try:
        while True:
            step()
            if ready() or other():
                break
    except RuntimeError as error:
        try:
            while True:
                step()
                if ready() or other():
                    break
        except ValueError as retry_error:
            step()
    finish(result)
    return result

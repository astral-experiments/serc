def f(values):
    try:
        for value in values:
            try:
                if accepted(value):
                    return value
            except LookupError:
                break
    except RuntimeError:
        pass
    finish()

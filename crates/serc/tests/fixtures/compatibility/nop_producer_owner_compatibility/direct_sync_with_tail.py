def f(manager, result):
    with manager:
        first()
        second()
        third()
        pass
    return result

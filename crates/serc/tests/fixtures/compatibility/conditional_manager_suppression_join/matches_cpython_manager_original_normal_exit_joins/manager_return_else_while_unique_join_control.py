def f(manager, mode, pending, step, finish):
    with manager():
        if mode:
            return
        else:
            while pending():
                step()
    finish()

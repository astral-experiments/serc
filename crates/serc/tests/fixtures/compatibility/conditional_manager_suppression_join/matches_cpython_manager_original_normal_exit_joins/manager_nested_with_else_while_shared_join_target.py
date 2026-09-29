def f(manager, inner, mode, pending, step, finish):
    with manager():
        if mode:
            with inner():
                while pending():
                    step()
        else:
            while pending():
                step()
    finish()

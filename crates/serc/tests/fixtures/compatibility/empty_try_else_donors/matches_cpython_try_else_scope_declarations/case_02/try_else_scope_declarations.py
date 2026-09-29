def outer():
    result = None
    def f():
        try:
            work()
        except Error:
            recover()
        else:
            nonlocal result
        finish()

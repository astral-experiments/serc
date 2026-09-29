def outer(__T):
    class C[__T]:
        callback = lambda: __T
    return C, __T

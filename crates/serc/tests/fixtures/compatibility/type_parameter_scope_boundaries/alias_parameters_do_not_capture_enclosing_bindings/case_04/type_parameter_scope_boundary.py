def outer(T):
    @decorate(lambda: T)
    class C[T]:
        def method(self):
            return T
    return C, T

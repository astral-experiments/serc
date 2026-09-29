def outer(T, value):
    class C[T]:
        callback = lambda: (T, value)
    return C, T

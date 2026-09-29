def outer(a):
    class C:
        a = type
        class D[T](*[a for i in (1,)]): pass
    return C

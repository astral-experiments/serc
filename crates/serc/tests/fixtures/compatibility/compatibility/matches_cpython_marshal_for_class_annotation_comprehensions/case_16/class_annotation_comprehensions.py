def outer(value):
    class C:
        type A = [value + i for i in (1, 2)]
    return C

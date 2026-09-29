def outer(value):
    class C:
        value = 10
        type A = [value + i for i in (1,)]
    return C

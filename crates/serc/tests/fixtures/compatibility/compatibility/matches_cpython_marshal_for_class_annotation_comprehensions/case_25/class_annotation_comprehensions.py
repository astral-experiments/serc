def outer(value):
    class C:
        value = 10
        x: [value + i for i in (1,)]
    return C

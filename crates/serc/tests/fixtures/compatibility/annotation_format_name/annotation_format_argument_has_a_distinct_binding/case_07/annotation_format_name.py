def outer(format):
    class C:
        value: format
    return C

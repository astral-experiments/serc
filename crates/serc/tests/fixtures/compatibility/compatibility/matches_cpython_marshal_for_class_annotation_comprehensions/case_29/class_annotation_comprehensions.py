def outer(__class__):
    class C:
        type A = [i for i in __class__]
    return C

def outer():
    global PlainAlias, GenericAlias, GenericFunction, GenericClass

    type PlainAlias = int
    type GenericAlias[T: int] = tuple[T, T]

    def GenericFunction[T: int]():
        pass

    class GenericClass[T: int]:
        pass

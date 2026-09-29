def outer():
    value = int

    class Nested[T]:
        item: value

    return Nested


class Generic[T]:
    def method[U](self):
        return T, U


class ____PrivateContainer:
    def outer(self):
        __value = int

        def generic[T: __value]() -> __value:
            return T, __value

        return generic

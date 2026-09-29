class Outer[T]:
    class Inner:
        function = lambda: None
        values = (value for value in ())

        def method(self, value: T) -> T:
            return value

        def generic[U](self, value: U) -> U:
            return value

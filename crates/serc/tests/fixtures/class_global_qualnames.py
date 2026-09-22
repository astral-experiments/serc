class Parent:
    global function, Child, generic

    def function(value: int):
        pass

    class Child:
        def method(self):
            pass

    def generic[T](value: T):
        pass

    def local(self):
        pass

    class Local:
        pass


class Outer:
    class Inner:
        global nested_function, NestedChild

        async def nested_function():
            pass

        class NestedChild:
            pass


def enclosing():
    class Parent:
        global enclosed_function

        def enclosed_function():
            pass

        def local(self):
            pass

    return Parent

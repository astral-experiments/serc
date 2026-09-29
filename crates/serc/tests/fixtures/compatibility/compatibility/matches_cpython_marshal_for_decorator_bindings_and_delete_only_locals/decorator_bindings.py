def bindings():
    @decorator
    def function():
        pass

    @(assigned := decorator)
    class Class:
        pass

    del deleted
    decorator = identity

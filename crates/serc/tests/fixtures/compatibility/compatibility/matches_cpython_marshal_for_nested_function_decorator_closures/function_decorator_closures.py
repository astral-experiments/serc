def method_decorators(decorator, decorator_factory):
    class Container:
        @decorator
        def method(self):
            pass

        @classmethod
        @decorator
        def class_method(cls):
            pass

        @decorator
        @staticmethod
        def static_method():
            pass

        @decorator_factory(1)
        async def coroutine(self):
            pass

    return Container


def nested_function_decorator(decorator):
    def parent():
        @decorator
        def child():
            pass

        return child

    return parent


def shadowed_class_decorator(decorator, local):
    class Container:
        decorator = local

        @decorator
        def method(self):
            pass

    return Container

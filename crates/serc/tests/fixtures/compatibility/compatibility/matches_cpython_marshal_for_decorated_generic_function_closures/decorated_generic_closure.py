class Container:
    @decorator
    def method[T](self, value: T) -> T: ...

def parent():
    captured = object()
    @decorator
    async def function[T: captured](value: T) -> T: ...
    return function

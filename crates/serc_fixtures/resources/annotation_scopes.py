from __future__ import annotations

def outer():
    from . import dependency

    class Generic[T: dependency.Type](dependency.Base):
        pass

    def function[T = dependency.Type](value: T):
        pass

    type Alias[T: dependency.Type] = tuple[dependency.Type, T]
    type Plain = dependency.Type


class Container:
    Local = object

    class Nested[T: Local](Local):
        pass


class CapturesBase[T](factory(lambda: T), metaclass=factory(lambda: T)):
    pass

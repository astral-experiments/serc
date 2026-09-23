def outer(_C__x):
    class C:
        def method(self):
            return __x
    return C


assert outer(42)().method() == 42


def class_shadow(_C__x):
    class C:
        __x = 99
        def method(self):
            return __x
    return C


assert class_shadow(42)().method() == 42


def method_shadow(_C__x):
    class C:
        def method(self, __x):
            return __x
    return C


assert method_shadow(42)().method(99) == 99


def different_classes(_B__x):
    class A:
        class B:
            def method(self):
                return __x
    return A.B


assert different_classes(42)().method() == 42


def same_classes(_C__x):
    class C:
        class __C:
            def method(self):
                return __x
    return C._C__C


assert same_classes(42)().method() == 42


class C:
    def same_context(__x):
        class ____C:
            def method(self):
                return __x
        return ____C

    def different_context(__x, _B__x):
        class B:
            def method(self):
                return __x
        return B


assert C.same_context(42)().method() == 42
assert C.different_context(99, 42)().method() == 42


def underscores_only(__x):
    class ____:
        def method(self):
            return __x
    return ____


assert underscores_only(42)().method() == 42


def generic(_C__x):
    class C[__T]:
        def method(self):
            return __x, __T
    return C


Generic = generic(42)
assert Generic().method() == (42, Generic.__type_params__[0])


def annotated(_C__x):
    class C:
        field: __x
        if True:
            conditional: __x
        def method(self) -> __x:
            return __x
    return C


Annotated = annotated(int)
assert Annotated().method() is int
assert Annotated.__annotations__ == {"field": int, "conditional": int}
assert Annotated.method.__annotations__ == {"return": int}

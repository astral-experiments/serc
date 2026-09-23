_D__x = 9
_C__x = 42


class C:
    def parameter_alias(__x):
        _C__x = 1
        return __x

    def local_alias():
        __x = 1
        return _C__x

    def reverse_alias():
        _C__x = 1
        return __x

    def ordered_alias():
        if False:
            _C__x
        y = 2
        __x = 1
        return __x, y

    def slow_alias(__x):
        try:
            _C__x += 1
        finally:
            __x += 1
        return _C__x

    def nested_alias():
        __x = 1
        def inner():
            return _C__x
        return inner()

    def captured_parameter(__x):
        return (lambda: _C__x)()

    def reverse_capture():
        _C__x = 1
        return (lambda: __x)()

    def deleted_alias(__x):
        del _C__x
        try:
            return __x
        except UnboundLocalError:
            return 1

    def comprehension_alias(__x):
        values = [_C__x for __x in (2, 3)]
        return __x, values

    def annotation_alias():
        __x: int
        try:
            return _C__x
        except UnboundLocalError:
            return 1

    def annotation_order():
        __x: int
        y = 1
        _C__x = 2
        return y, __x

    def captured_order():
        __x = 1
        _D = 2
        return (lambda: (_C__x, _D))()

    def double_target(__x):
        values = [(__x, _C__x) for __x, _C__x in ((1, 2),)]
        return __x, values

    def unrelated_class():
        _C__x = 1
        class D:
            def read():
                return __x
        return D.read()


    def comprehension_keeps_global():
        values = [__x for __x in (1,)]
        return _C__x, values

    def branch_alias(flag):
        if flag:
            __x = 1
        else:
            _C__x = 2
        return __x

    def repeated_alias_comprehensions():
        __x = 1
        first = [(lambda: __x) for __x in (2,)]
        middle = 3
        second = [(lambda: _C__x) for _C__x in (4,)]
        return __x, first[0](), second[0](), middle

    def mixed_private_cells():
        __z = 1
        __a = 2
        return (lambda: (_C__z, __a))()

    def named_mixed_private_cells():
        __z = 1
        __a = 2
        def inner():
            return _C__z, __a
        return inner()

    def captured_single_target(__x):
        values = [(lambda: _C__x)() for __x in (2, 3)]
        return __x, values

    def captured_double_target(__x):
        values = [(lambda: _C__x)() for __x, _C__x in ((1, 2),)]
        return __x, values

    def free_aliases(__x):
        _C__x = __x
        def inner():
            return __x, _C__x
        return inner

    def lambda_free_aliases(__x):
        _C__x = __x
        return lambda: (__x, _C__x)

    def forwarded_free_aliases(__x):
        _C__x = __x
        def inner():
            aliases = __x, _C__x
            return aliases, lambda: __x
        return inner()

    def nonlocal_read():
        __x = 1
        def inner():
            nonlocal _C__x
            return __x
        return inner()

    def lambda_assignment():
        return (lambda: ((__x := 42), _C__x)[1])()

    def lambda_parameter():
        return (lambda __x: _C__x)(42)

    def lambda_reverse_parameter():
        return (lambda _C__x: __x)(42)

    def lambda_keyword_only():
        return (lambda *, __x=42: _C__x)()

    def lambda_varargs():
        return (lambda *__x: _C__x)(42)

    def lambda_kwargs():
        return (lambda **__x: _C__x)(answer=42)

    def lambda_capture():
        return (lambda __x: lambda: _C__x)(42)()

    def lambda_shadow():
        return (lambda __x: lambda _C__x: __x)(1)(42)

    def lambda_no_outer_capture(_C__x):
        return (lambda __x: _C__x)(42)

    def lambda_nested_default():
        return (lambda __x: lambda y=_C__x: y)(42)()

    def lambda_keyword_default():
        __x = 42
        return (lambda *, __x=_C__x: _C__x)()


assert C.parameter_alias(42) == 1
assert C.local_alias() == 1
assert C.reverse_alias() == 1
assert C.ordered_alias() == (1, 2)
assert C.slow_alias(40) == 42
assert C.nested_alias() == 1
assert C.captured_parameter(42) == 42
assert C.reverse_capture() == 1
assert C.deleted_alias(42) == 1
assert C.comprehension_alias(42) == (42, [2, 3])
assert C.annotation_alias() == 1
assert C.annotation_order() == (1, 2)
assert C.captured_order() == (1, 2)
assert C.double_target(42) == (42, [(2, 2)])
assert C.unrelated_class() == 9
assert C.mixed_private_cells() == (1, 2)
assert C.captured_double_target(42) == (42, [2])
assert C.nonlocal_read() == 1
assert C.lambda_assignment() == 42
assert C.lambda_parameter() == 42
assert C.lambda_reverse_parameter() == 42
assert C.lambda_keyword_only() == 42
assert C.lambda_varargs() == (42,)
assert C.lambda_kwargs() == {"answer": 42}
assert C.lambda_capture() == 42
assert C.lambda_shadow() == 42
assert C.lambda_no_outer_capture(1) == 42
assert not C.lambda_no_outer_capture.__code__.co_cellvars
assert C.lambda_nested_default() == 42
assert C.lambda_keyword_default() == 42
assert C.captured_single_target(42) == (42, [2, 3])
assert C.named_mixed_private_cells() == (1, 2)
assert C.branch_alias(True) == 1
assert C.branch_alias(False) == 2
assert C.repeated_alias_comprehensions() == (1, 2, 4, 3)
assert C.comprehension_keeps_global() == (42, [1])


def comprehension_owns_alias(_C__x):
    class C:
        def read():
            return [_C__x for __x in (1,)]

        def read_iterable():
            return [_C__x for __x in _C__x]
    return C


assert comprehension_owns_alias(42).read() == [1]
assert not comprehension_owns_alias(42).read.__code__.co_freevars
assert comprehension_owns_alias((1, 2)).read_iterable() == [1, 2]
assert C.free_aliases(42)() == (42, 42)
assert C.free_aliases(42).__code__.co_freevars == ("_C__x",)
assert C.lambda_free_aliases(42)() == (42, 42)
assert C.lambda_free_aliases(42).__code__.co_freevars == ("_C__x",)
assert C.forwarded_free_aliases(42)[0] == (42, 42)
assert C.forwarded_free_aliases(42)[1]() == 42

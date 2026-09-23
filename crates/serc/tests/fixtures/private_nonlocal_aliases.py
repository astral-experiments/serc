class C:
    def explicit_declaration():
        __x = 1
        def update():
            nonlocal _C__x
            __x = 2
        update()
        return __x

    def raw_declaration():
        _C__x = 1
        def update():
            nonlocal __x
            _C__x = 2
        update()
        return _C__x

    def augmented():
        __x = 1
        def update():
            nonlocal _C__x
            __x += 1
        update()
        return __x

    def deleted():
        __x = 1
        def update():
            nonlocal _C__x
            del __x
        update()
        try:
            return __x
        except UnboundLocalError:
            return 2

    def forwarded():
        __x = 1
        def middle():
            def update():
                nonlocal _C__x
                __x = 2
            update()
        middle()
        return __x

    def parameter(__x):
        def update():
            nonlocal _C__x
            __x = 2
        update()
        return __x

    def same_spelling():
        __x = 1
        def update():
            nonlocal __x
            __x = 2
        update()
        return __x

    def ordinary():
        x = 1
        def update():
            nonlocal x
            x = 2
        update()
        return x

    def unrelated_prefix():
        _D__x = 1
        def update():
            nonlocal _D__x
            __x = 99
            _D__x = 2
            return __x
        result = update()
        return _D__x, result

    def comprehension():
        __x = 41
        def update():
            nonlocal _C__x
            values = [__x for __x in (1, 2)]
            __x += 1
            return values
        result = update()
        return __x, result

    def captured_comprehension():
        __x = 41
        def update():
            nonlocal _C__x
            values = [(lambda: __x)() for __x in (1, 2)]
            __x += 1
            return values
        result = update()
        return __x, result


class _C:
    def leading_underscore():
        __x = 1
        def update():
            nonlocal _C__x
            __x = 2
        update()
        return __x


class ___:
    def unmangled():
        __x = 1
        def update():
            nonlocal __x
            __x = 2
        update()
        return __x


assert C.explicit_declaration() == 2
assert C.raw_declaration() == 2
assert C.augmented() == 2
assert C.deleted() == 2
assert C.forwarded() == 2
assert C.parameter(1) == 2
assert C.same_spelling() == 2
assert C.ordinary() == 2
assert C.unrelated_prefix() == (2, 99)
assert C.comprehension() == (42, [1, 2])
assert C.captured_comprehension() == (41, [1, 2])
assert _C.leading_underscore() == 2
assert ___.unmangled() == 2

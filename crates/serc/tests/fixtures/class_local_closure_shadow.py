x = "global"


def bound(x):
    class C:
        before = x
        x = "class"
        present = x

        def method(self):
            return x

        del x
        after = x

    return C


C = bound("enclosing")
assert C.before == "global"
assert C.present == "class"
assert C.after == "global"
assert C().method() == "enclosing"


def captured(x):
    class C:
        observed = x

        def method(self):
            return x

    return C


C = captured("enclosing")
assert C.observed == "enclosing"
assert C().method() == "enclosing"


def explicit_global(x):
    class C:
        global x
        observed = x

        def method(self):
            return x

    return C


C = explicit_global("enclosing")
assert C.observed == "global"
assert C().method() == "enclosing"


def explicit_nonlocal(x):
    class C:
        nonlocal x
        x = "nonlocal"
        observed = x

        def method(self):
            return x

    return C, x


C, value = explicit_nonlocal("enclosing")
assert value == "nonlocal"
assert C.observed == "nonlocal"
assert C().method() == "nonlocal"


def annotated(x):
    class C:
        x = str
        field: x

        def method(self, value: x) -> x:
            return x

    return C


C = annotated(int)
assert C.__annotations__["field"] is str
assert C.method.__annotations__["value"] is str
assert C.method.__annotations__["return"] is str
assert C().method(None) is int


def annotation_only(x):
    class C:
        x: str
        observed = x

        def method(self):
            return x

    return C


C = annotation_only("enclosing")
assert C.observed == "global"
assert C().method() == "enclosing"


def comprehensions(x):
    class C:
        x = ["class"]
        first_iterable = [item for item in x]
        body = [x for _ in (0,)]
        empty_target = [x for () in [()]]

        def method(self):
            return x

    return C


C = comprehensions("enclosing")
assert C.first_iterable == ["class"]
assert C.body == ["enclosing"]
assert C.empty_target == ["enclosing"]
assert C().method() == "enclosing"


def comprehension_target(x):
    class C:
        items = [x for x in ()]
        observed = x

        def method(self):
            return x

    return C


C = comprehension_target("enclosing")
assert C.observed == "enclosing"
assert C().method() == "enclosing"

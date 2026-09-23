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

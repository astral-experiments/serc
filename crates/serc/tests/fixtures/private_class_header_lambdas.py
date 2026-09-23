def unbound_private_header_lambda():
    class Outer:
        class Container[T: (lambda: __key)]:
            pass
    return Outer


unbound_private_header_lambda()


def mixed_private_header_lambda(__key, _Outer__key):
    class Outer:
        body = __key
        class Container[T: (lambda: __key)]:
            pass
    return Outer


MixedLambda = mixed_private_header_lambda(41, 42)
assert MixedLambda.body == 42
assert MixedLambda.Container.__type_params__[0].__bound__() == 41

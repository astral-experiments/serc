def function_headers(base, metaclass, bases, options, captured):
    def parent():
        class Direct(base, metaclass=metaclass):
            pass

        class Expanded(*bases, **options):
            pass

        class Nested((lambda: captured)()):
            pass

        return Direct, Expanded, Nested

    return parent


def class_headers(base, metaclass, bases, options):
    class Container:
        class Direct(base, metaclass=metaclass):
            pass

        class Expanded(*bases, **options):
            pass

    return Container


def local_headers(base, metaclass):
    def parent(base, metaclass):
        class Child(base, metaclass=metaclass):
            pass

        return Child

    return parent


def shadowed_class_headers(base, metaclass, local_base, local_meta):
    class Container:
        base = local_base
        metaclass = local_meta

        class Child(base, metaclass=metaclass):
            pass

    return Container


def generic_headers(base, metaclass, bound, default):
    def parent():
        class Child[T: bound = default](base, metaclass=metaclass):
            value: T

        return Child

    return parent


def generic_type_parameter_shadow(T):
    def parent():
        class Child[T](T):
            pass

        return Child

    return parent


class Global(global_base, metaclass=global_meta):
    pass

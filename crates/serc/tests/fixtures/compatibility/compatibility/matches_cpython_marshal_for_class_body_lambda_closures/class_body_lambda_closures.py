def self_reference():
    class Local:
        factory = lambda: Local

    return Local


def nested_self_reference():
    class Local:
        factory = lambda: (lambda: Local)

    return Local


def enclosing_reference():
    captured = object()

    class Local:
        factory = lambda: captured

    return Local


def class_local_shadow():
    class Local:
        Local = object()
        factory = lambda: Local

    return Local


def method_control():
    class Local:
        def factory(self):
            return Local

    return Local


def direct_body_control():
    class Local:
        value = Local

    return Local


def function_global_control():
    global GlobalLocal

    class GlobalLocal:
        factory = lambda: GlobalLocal

    return GlobalLocal


def class_global_control():
    captured = object()

    class Local:
        global captured
        factory = lambda: captured

    return Local


def class_nonlocal_control():
    captured = object()

    class Local:
        nonlocal captured
        factory = lambda: captured

    return Local


def dunder_class_control():
    class Local:
        factory = lambda: __class__

    return Local


class ModuleControl:
    factory = lambda: ModuleControl


class ClassParentControl:
    class Local:
        factory = lambda: Local

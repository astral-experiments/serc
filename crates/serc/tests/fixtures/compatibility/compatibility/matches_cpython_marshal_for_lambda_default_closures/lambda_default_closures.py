def class_self_default():
    class Local:
        thunk = lambda value=Local: value

    return Local


def class_enclosing_default():
    captured = object()

    class Local:
        thunk = lambda value=captured: value

    return Local


def class_nested_lambda_default():
    class Local:
        thunk = lambda value=(lambda: Local): value

    return Local


def function_enclosing_default(captured):
    def parent():
        thunk = lambda value=captured: value
        return thunk

    return parent


def nested_lambda_body_default(captured):
    return lambda: (lambda value=captured: value)


def nested_lambda_nested_default(captured):
    return lambda: (lambda value=(lambda: captured): value)


def nested_lambda_generator_default(captured):
    return lambda: (lambda value=(captured for _ in ()): value)


def nested_lambda_local_default():
    return lambda captured: (lambda value=captured: value)


def class_local_before_control():
    captured = object()

    class Local:
        captured = object()
        thunk = lambda value=captured: value

    return Local


def class_local_after_control():
    captured = object()

    class Local:
        thunk = lambda value=captured: value
        captured = object()

    return Local


def class_body_control():
    class Local:
        thunk = lambda: Local

    return Local


def function_global_control():
    global GlobalLocal

    class GlobalLocal:
        thunk = lambda value=GlobalLocal: value

    return GlobalLocal


class ModuleControl:
    thunk = lambda value=ModuleControl: value


class ClassParentControl:
    class Local:
        thunk = lambda value=Local: value


def default_parameter_name_control():
    class Local:
        thunk = lambda first, second=first: second

    return Local

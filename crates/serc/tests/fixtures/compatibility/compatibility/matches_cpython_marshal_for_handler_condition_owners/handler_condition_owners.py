def named_handler_condition_boolean_call(action, value):
    try:
        action(value)
    except RuntimeError as error:
        if value.ready():
            action(str(error) or type(error).__name__)
        action(error)
        raise


def named_handler_prior_return_condition(action, value):
    try:
        action(value)
    except RuntimeError as error:
        if value.ready():
            return value
        if not value:
            raise
        raise ValueError() from error


def named_handler_prior_return_method_condition(action, value):
    try:
        action(value)
    except RuntimeError as error:
        if value.ready():
            return value
        if not value.valid():
            raise
        raise ValueError() from error


def named_handler_prior_return_boolean_condition(action, value):
    try:
        action(value)
    except RuntimeError as error:
        if value.ready():
            return value
        if value.valid() or value.other():
            raise
        raise ValueError() from error


def bare_handler_prior_return_condition(action, value):
    try:
        action(value)
    except RuntimeError:
        if value.ready():
            return value
        if not value:
            raise
        raise ValueError()


def named_handler_prior_return_name_call_control(action, value, probe):
    try:
        action(value)
        return value
    except RuntimeError as error:
        if value:
            action(error)
            return value
        if probe(str(error)):
            raise
        if not probe(error):
            raise
        action(error)
    action(value)

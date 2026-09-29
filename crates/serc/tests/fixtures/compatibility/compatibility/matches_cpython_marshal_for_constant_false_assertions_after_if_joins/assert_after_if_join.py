def after_if_pass(flag, message):
    if flag:
        pass
    assert False, message


def after_if_return(flag, message):
    if flag:
        return
    assert False, message


def after_if_raise(flag, error, message):
    if flag:
        raise error
    assert False, message


def after_if_else(flag, message):
    if flag:
        first()
    else:
        second()
    assert False, message


def after_if_chain(first, second, message):
    if first:
        one()
    elif second:
        two()
    assert False, message


def assert_alone(message):
    assert False, message


def after_expression(message):
    consume()
    assert False, message


def after_assignment(message):
    value = produce()
    assert False, message


def dynamic_after_if(flag, condition, message):
    if flag:
        consume()
    assert condition, message

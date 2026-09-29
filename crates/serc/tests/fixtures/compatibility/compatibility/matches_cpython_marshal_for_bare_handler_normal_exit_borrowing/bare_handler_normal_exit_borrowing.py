def bare_pass(value):
    try:
        action()
    except:
        pass
    consume(value)


def bare_assignment(value):
    try:
        result = action()
    except:
        result = None
    consume(value, result)


def branch_local_tail(value, enabled):
    if enabled:
        try:
            action()
        except:
            pass
        consume(value)
    finish()


def loop_local_tail(value, items):
    for item in items:
        try:
            action(item)
        except:
            pass
        consume(value)


def typed_handler_control(value):
    try:
        action()
    except Error:
        pass
    consume(value)


def direct_name_return(value):
    try:
        action()
    except:
        pass
    return value


def shared_branch_join(value, enabled):
    if enabled:
        try:
            action()
        except:
            pass
    consume(value)


def shared_loop_join(value, items):
    for item in items:
        try:
            action(item)
        except:
            pass
    consume(value)

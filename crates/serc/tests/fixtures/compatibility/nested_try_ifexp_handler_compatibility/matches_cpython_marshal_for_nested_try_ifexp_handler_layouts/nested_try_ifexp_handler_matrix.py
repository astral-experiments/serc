def name_target_outer_tail(check, work, make):
    response = make()
    try:
        if not check(work()):
            return response.done()
        try:
            value = work()
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
    except Exception as error:
        result = error
    response.result = result
    return response.done()


def attribute_target_outer_tail(check, work, make):
    response = make()
    try:
        value = work()
        try:
            value = work()
            response.result = "OK" if check(value) else value
        except Exception as error:
            response.result = error
    except Exception as error:
        response.result = error
    return response.done()


def non_tail_inner_try(check, work, make):
    response = make()
    try:
        value = work()
        try:
            value = work()
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
        response.result = result
    except Exception as error:
        response.result = error
    return response.done()


def stale_dynamic_owner(check, work, make):
    response = make()
    try:
        value = work()
        try:
            value = work()
            value = (value, value)
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
    except Exception as error:
        result = error
    response.result = result
    return response.done()


def outer_no_successor(check, work):
    try:
        try:
            value = work()
            value = (value, value)
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
    except Exception as error:
        result = error


def outer_else_control(check, work):
    try:
        try:
            value = work()
            value = (value, value)
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
    except Exception as error:
        result = error
    else:
        work(result)
    return result


def unnamed_handler_control(check, work):
    try:
        try:
            value = work()
            value = (value, value)
            result = "OK" if check(value) else value
        except Exception:
            result = value
    except Exception as error:
        result = error
    return result


def non_call_test_control(check, work):
    try:
        try:
            value = work()
            value = (value, value)
            result = "OK" if value else value
        except Exception as error:
            result = error
    except Exception as error:
        result = error
    return result


def assignment_not_final_control(check, work):
    try:
        try:
            value = work()
            value = (value, value)
            result = "OK" if check(value) else value
            work(result)
        except Exception as error:
            result = error
    except Exception as error:
        result = error
    return result

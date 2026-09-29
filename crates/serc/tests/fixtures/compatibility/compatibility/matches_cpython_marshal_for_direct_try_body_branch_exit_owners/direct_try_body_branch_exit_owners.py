def direct_exit(marker, text):
    try:
        if marker in text:
            left, right = first()
        else:
            value = second()
            left, right = value.real, value.imag
    except Error:
        raise ValueError
    consume(left, right)


def direct_comparison_exit(array, dtype):
    try:
        if array.shape[1] > 0:
            consolidated = array.view(dtype)
        else:
            consolidated = empty(len(array), dtype=dtype)
    except TypeError as error:
        message = 'unsupported dtype'
        raise TypeError(message) from error
    consume(consolidated)


def direct_assignment_fallthrough(flag):
    try:
        if flag:
            value = first()
        else:
            value = second()
    except Error:
        value = fallback()
    consume(value)


def protected_join_control(marker, text):
    try:
        if marker in text:
            left, right = first()
        else:
            value = second()
            left, right = value.real, value.imag
        consume(left, right)
    except Error:
        raise ValueError


def try_else_control(flag):
    try:
        if flag:
            first()
        else:
            second()
    except Error:
        report()
    else:
        success()
    after()

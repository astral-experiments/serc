def bare_handler_loop_marker(values):
    try:
        action()
    except Error:
        for value in values:
            match = regex.search("value", value)
            if match:
                result = match.group("value")
            if result is not None:
                break
        return result


def ordinary_bare_handler_condition(value):
    try:
        action()
    except Error:
        if regex.search("value", value):
            recover()
        finish()


def protected_break_and_following_empty_handler(values):
    result = []
    try:
        for value in values:
            if value == "stop":
                break
            result.append(value)
        try:
            result.append(source.attribute())
        except AttributeError:
            pass
        consume(
            value00, value01, value02, value03, value04,
            value05, value06, value07, value08, value09,
            value10, value11, value12, value13, value14,
            value15, value16, value17, value18, value19,
            value20, value21, value22, value23, value24,
            value25, value26, value27, value28, value29,
            value30, value31, value32, value33, value34,
            value35, value36, value37, value38, value39,
        )
    except ImportError:
        pass
    return result


def triple_named_handlers_with_continuation():
    try:
        first()
    except Error as error:
        first_message = str(error)
        try:
            second()
        except Error as error:
            second_message = str(error)
            try:
                third()
                result = 3
            except Error as error:
                third_message = str(error)
                consume(first_message, second_message, third_message)
    normalized = normalize(result)
    consume(normalized)
    return normalized

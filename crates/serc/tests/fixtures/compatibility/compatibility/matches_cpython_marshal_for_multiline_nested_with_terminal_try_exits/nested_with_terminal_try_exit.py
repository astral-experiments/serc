def else_path(outer):
    with (
        outer() as value,
        value.span,
    ):
        try:
            result = action()
        except Error:
            raise
        else:
            return result


def fallthrough_handler(outer):
    with (
        outer() as value,
        value.span,
    ):
        try:
            action()
        except Error:
            recover()


def same_line_terminal(outer):
    with outer() as value, value.span:
        try:
            result = action()
        except Error:
            raise
        else:
            return result


def direct_return(outer):
    with (
        outer() as value,
        value.span,
    ):
        return action()

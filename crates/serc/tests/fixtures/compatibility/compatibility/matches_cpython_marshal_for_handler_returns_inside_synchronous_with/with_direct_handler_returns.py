def direct(manager):
    with manager:
        try:
            return produce()
        except Error:
            return recover()


def fully_terminating(manager, value, output):
    with manager:
        if is_array(value):
            if output is None:
                return first()
            else:
                return second()
        elif output is None:
            try:
                return typed()
            except AttributeError:
                return fallback()
        else:
            return third()


def enclosing_try_control(manager):
    try:
        with manager:
            return produce()
    except Error:
        return recover()


def fallthrough(manager):
    with manager:
        try:
            produce()
        except Error:
            return False
        return True


def recovery_then_return(manager):
    with manager:
        try:
            produce()
        except Error:
            recover()
            return False
        return True

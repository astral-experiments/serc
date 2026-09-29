def direct_none(action):
    try:
        outer()
    except TypeError:
        try:
            action()
            raise AssertionError()
        except TypeError:
            return


def direct_value(action):
    try:
        outer()
    except TypeError:
        try:
            action()
            raise AssertionError()
        except ValueError:
            return False
        except TypeError:
            return True


def top_level(action):
    try:
        action()
    except TypeError:
        return


def nested_fallthrough(action):
    try:
        outer()
    except TypeError:
        try:
            action()
        except TypeError:
            recover()

def direct(manager):
    try:
        with manager():
            return True
    except Error:
        return False


def arguments(manager, first, second):
    try:
        with manager((first, second), timeout=1.0):
            return True
    except Error:
        return False


def attribute(manager, first, second):
    try:
        with manager.open((first, second), timeout=1.0):
            return True
    except Error:
        return False


def named_control(manager, first, second):
    try:
        with manager.open((first, second), timeout=1.0) as value:
            return value
    except Error:
        return False


def preceding_call(manager, first, second):
    try:
        with manager.open((first, second), timeout=1.0):
            manager.observe()
            return True
    except Error:
        return False

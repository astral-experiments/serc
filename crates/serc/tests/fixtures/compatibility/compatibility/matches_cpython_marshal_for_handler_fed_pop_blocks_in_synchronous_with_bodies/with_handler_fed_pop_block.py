def named(manager):
    with manager:
        try:
            raise Other
        except Error as error:
            recover(error)

def unnamed(manager):
    with manager:
        try:
            raise Error
        except Error:
            recover()

def assertion(manager, condition):
    with manager:
        try:
            raise Other
        except Error:
            assert condition

def body_falls_through(manager):
    with manager:
        try:
            action()
        except Error:
            recover()

def terminal_loop(manager):
    with manager:
        try:
            while True:
                action()
        except Error:
            pass

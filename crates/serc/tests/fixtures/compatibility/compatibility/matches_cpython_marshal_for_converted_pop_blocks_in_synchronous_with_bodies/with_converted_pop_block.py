def terminal_handler(manager):
    with manager:
        try:
            action()
        except Error:
            raise

def terminal_else(manager):
    with manager:
        try:
            action()
        except Error:
            recover()
        else:
            raise Other

def nested_return(outer, inner):
    with outer:
        with inner:
            return value

def nested_raise(outer, inner):
    with outer:
        with inner:
            raise Error

def no_duplicate_after_finally(manager):
    with manager:
        try:
            action()
        finally:
            cleanup()

def no_duplicate_after_if(manager, condition):
    with manager:
        try:
            action()
            if condition:
                other()
        except Error:
            raise

def multiple_normal_predecessors(manager):
    with manager:
        try:
            action()
        except Error:
            recover()

def terminal_named_handler(value):
    try:
        work()
    except RuntimeError as error:
        if value:
            pass
        else:
            raise

def named_handler_with_following_code(value):
    try:
        work()
        update()
    except RuntimeError as error:
        report(error)
        if value:
            recover()
            result = value
        else:
            raise
    finish()

def bare_handler_with_following_code(value):
    try:
        work()
    except RuntimeError:
        if value:
            recover()
        else:
            raise
    finish()

def generator_handler(value):
    yield value
    try:
        work()
    except RuntimeError as error:
        if value:
            recover()
        else:
            raise

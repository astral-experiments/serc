def is_none(value):
    try:
        action()
    except Error:
        if value is None:
            recover()
        finish()


def is_not_none(value, items):
    try:
        action()
    except Error:
        if value is not None:
            recover()
        result = [item for item in items]


def named(value):
    try:
        action()
    except Error as error:
        if value is None:
            recover(error)
        finish()


def nested(value):
    try:
        action()
    except Error:
        if predicate(value):
            if value is not None:
                recover()
        finish()


def reversed_control(value):
    try:
        action()
    except Error:
        if None is value:
            recover()
        finish()


def outside_handler_control(value):
    if value is None:
        recover()
    finish()


def later_bare_handler_control(value):
    try:
        action()
    except ValueError as error:
        consume(error)
    except TypeError:
        if value is None:
            recover()
        finish()

def raised(outer, first, second):
    if outer:
        if first and second:
            raise Error
    else:
        fallback()
    after()

def returned(outer, errors, value):
    if outer:
        if not errors and value is not None:
            return value
    elif value:
        fallback()
    after()

def multiline(outer, first, second):
    if outer:
        if (
            first
            and second
        ):
            raise Error
    else:
        fallback()
    after()

def unique_false_control(outer, first, second):
    if outer:
        if first or second:
            raise Error
    else:
        fallback()
    after()

def simple_control(outer, inner):
    if outer:
        if inner:
            raise Error
    else:
        fallback()
    after()

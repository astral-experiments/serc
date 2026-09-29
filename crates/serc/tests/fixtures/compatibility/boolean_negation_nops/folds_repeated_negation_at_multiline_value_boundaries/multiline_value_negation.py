def standalone(flag, other, value):
    return (
        not not value
    )

def and_value(flag, other, value):
    return (
        not flag and (not not value)
    )

def or_assignment(flag, other, value):
    result = (
        not flag or (not not value)
    )
    return result

def call_value(flag, other, value):
    return work((
        not not value
    ))

def possibly_unbound(flag, other):
    if flag:
        value = other
    return (
        not not value
    )

def same_line_control(flag, other, value):
    return not not value

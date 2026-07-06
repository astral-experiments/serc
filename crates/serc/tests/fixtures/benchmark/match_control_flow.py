match foo:
    case foo_bar: ...
match foo:
    case _: ...
match 1:
    case _ if (True): ...
match (1, 2):
    case _: ...
match subject:
    case [a, b]: ...
    case (a, b): ...
match value:
    case 1:
        h(x)
    case _:
        ...
def terminal_try_star():
    try:
        assigned = 1
    except* ValueError:
        assigned = 2
try: ...
except* ValueError: ...
raise terminal
match subject:
    case unreachable_capture:
        pass
[temporary for temporary in values]

def match_assignment(value):
    match value:
        case True:
            assigned = 1
        case False:
            assigned = 2
    return assigned

def subject_before_capture(value):
    match match:
        case case:
            pass
    match = value
    return match, case

def terminal_default_if(value):
    match value:
        case 1:
            pass
        case _:
            if value != 2:
                pass
            elif value == 2:
                pass

def terminal_nondefault_if(value):
    match value:
        case 1:
            if value != 2:
                pass
            elif value == 2:
                pass
        case _:
            pass

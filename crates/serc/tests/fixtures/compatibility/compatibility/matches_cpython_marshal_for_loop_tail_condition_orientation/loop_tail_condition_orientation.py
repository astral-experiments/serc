def terminal_while_break():
    while True:
        value = step()
        if stop(value):
            break
        if condition(value):
            consume(value)

def terminal_while_compound_control():
    while True:
        value = step()
        if stop(value):
            break
        if condition(value):
            consume(value)
        else:
            reject(value)

def nonfinal_nested_while(items):
    for item in items:
        if first(item):
            while condition(item):
                consume(item)
        elif second(item):
            other(item)

def direct_nested_while_control(items):
    for item in items:
        while condition(item):
            consume(item)

def protected_nested_while_control(items):
    try:
        for item in items:
            while condition(item):
                consume(item)
    except Exception:
        recover()

def nonfinal_assertions(items):
    for item in items:
        if first(item):
            assert condition(item)
        else:
            assert other(item)

def nonfinal_break(items):
    for item in items:
        if first(item):
            if stop(item):
                break
        else:
            consume(item)

def nonfinal_raise(items):
    for item in items:
        if first(item):
            if rejected(item):
                raise ValueError(item)
        else:
            consume(item)

def nonfinal_return(items):
    for item in items:
        if first(item):
            if accepted(item):
                return item
        else:
            consume(item)

def direct_assertion_control(items):
    for item in items:
        assert condition(item)

def two_operands(manager, value, touch):
    with manager:
        if value.ready and value is not None:
            touch(value)

def three_operands(manager, value, touch):
    with manager:
        if value.ready and value is not None and value.active:
            touch(value)

def nested_body(manager, value, fallback, touch):
    with manager:
        if value.ready and value is not None and value.active:
            if fallback(value):
                touch(value)

def none_last(manager, value, fallback, touch):
    with manager:
        if value.ready and value.active and value is not None:
            if fallback(value):
                touch(value)

def disjunction(manager, value, fallback, touch):
    with manager:
        if value.ready or value is None or value.active:
            if fallback(value):
                touch(value)

def prior_condition(manager, value, flag, fallback, touch):
    with manager:
        if flag:
            touch(value)
        if value.ready and value is not None and value.active:
            if fallback(value):
                touch(value)

def following_statement(manager, value, touch):
    with manager:
        if value.ready and value is not None and value.active:
            touch(value)
        touch(value)

def nested_manager(first, second, value, fallback, touch):
    with first:
        with second:
            if value.ready and value is not None and value.active:
                if fallback(value):
                    touch(value)

def try_around_with(manager, value, fallback, touch):
    try:
        with manager:
            if value.ready and value is not None and value.active:
                if fallback(value):
                    touch(value)
    except Error:
        pass

def none_first_control(manager, value, touch):
    with manager:
        if value is not None and value.ready and value.active:
            touch(value)

def loop_control(manager, values, touch):
    with manager:
        for value in values:
            if value.ready and value is not None and value.active:
                touch(value)

def nested_try_control(manager, value, touch):
    with manager:
        try:
            if value.ready and value is not None and value.active:
                touch(value)
        except Error:
            pass

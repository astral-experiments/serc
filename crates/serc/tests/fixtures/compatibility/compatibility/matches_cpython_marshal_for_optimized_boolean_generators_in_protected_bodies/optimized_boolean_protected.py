def try_assignment(values, touch):
    try:
        result = any(value.ready for value in values)
        touch(result)
    except Error:
        pass

def try_condition(values, touch):
    try:
        if any(value.ready for value in values):
            touch(values)
    except Error:
        pass

def try_boolean_conditions(values, state, touch):
    try:
        if state.active and any(value.ready for value in values):
            touch(state)
        if state.active or all(value.ready for value in values):
            touch(state)
    except Error:
        pass

def try_finally(values, touch, cleanup):
    try:
        if any(value.ready for value in values):
            touch(values)
    finally:
        cleanup()

def try_loop(groups, touch):
    try:
        for values in groups:
            result = any(value.ready for value in values)
            touch(result)
    except Error:
        pass

def with_direct(manager, values, touch):
    with manager:
        result = any(value.ready for value in values)
        touch(result)

def condition_before_with(manager, values, flag, touch):
    if flag:
        touch(flag)
    with manager:
        result = any(value.ready for value in values)
        touch(result)

def with_boolean_assignment(manager, values, state, touch):
    with manager:
        result = state.active and any(value.ready for value in values)
        touch(result)

def with_boolean_condition(manager, values, state, touch):
    with manager:
        if state.active or all(value.ready for value in values):
            touch(state)

def with_prior_condition(manager, values, state, touch):
    with manager:
        if state.active:
            touch(state)
        result = any(value.ready for value in values)
        touch(result)

def with_loop(manager, groups, touch):
    with manager:
        for values in groups:
            result = any(value.ready for value in values)
            touch(result)

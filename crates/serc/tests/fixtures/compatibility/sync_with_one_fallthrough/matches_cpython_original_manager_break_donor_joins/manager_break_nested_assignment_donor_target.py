def run(values, outer, inner, state):
    for value in values:
        with outer(value), inner(value):
            if not value:
                break
            else:
                state.result = value
    return state

def select(groups, cleanup, state):
    try:
        for group in groups:
            try:
                validate(group)
                if group is None:
                    return state.missing
                for item in group:
                    if item:
                        return state.found
            finally:
                cleanup(group)
    except ValueError:
        return state.failed
    return state.empty

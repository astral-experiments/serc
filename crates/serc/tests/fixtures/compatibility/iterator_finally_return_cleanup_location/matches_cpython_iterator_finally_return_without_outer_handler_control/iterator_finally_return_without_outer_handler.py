def select(groups, cleanup, state):
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
    return state.empty

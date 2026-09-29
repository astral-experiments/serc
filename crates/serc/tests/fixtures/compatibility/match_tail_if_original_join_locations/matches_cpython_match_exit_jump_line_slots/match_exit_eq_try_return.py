def f(subject, state, flag):
    match subject:
        case Kind.OTHER:
            return None
        case Kind.WANTED:
            if state is not None:
                try:
                    return convert(state.value)
                except ValueError:
                    pass
    return None

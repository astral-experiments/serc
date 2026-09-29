def update(callback, report, types, state, displayed):
    if displayed:
        try:
            callback()
        except Exception as error:
            if types:
                matches = [index for index, kind in enumerate(types) if isinstance(error, kind)]
                if matches:
                    state[0] = matches[0] + 1
                else:
                    report(error)
            else:
                report(error)
    return finish(displayed)

def target(items, manager, work, report):
    for item in items:
        try:
            with manager as value:
                result = work(value)
                return result
            break
        except ValueError as error:
            report(error)
    return None

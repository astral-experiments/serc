def target(groups, manager, stop):
    try:
        for group in groups:
            with manager(group) as values:
                for value in values:
                    consume(value)
                    if stop(value):
                        return
    finally:
        finish()

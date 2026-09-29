def target(groups, manager, stop):
    try:
        for group in groups:
            for value in group:
                with manager(value):
                    if stop(value):
                        return
    finally:
        finish()

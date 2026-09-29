def run(values, manager, active):
    try:
        work()
    finally:
        for value in values:
            if active:
                with manager:
                    release(value)
                continue

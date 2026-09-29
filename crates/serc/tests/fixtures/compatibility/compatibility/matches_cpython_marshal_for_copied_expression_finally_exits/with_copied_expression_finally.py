def expression_compare(manager, flag):
    with manager:
        try:
            work()
        finally:
            if flag is True:
                cleanup()

def retained_unconditional(manager):
    with manager:
        try:
            work()
        finally:
            cleanup()

def protected_break(manager, active, stop):
    with manager:
        try:
            while active:
                if stop:
                    break
                step()
        except Error:
            cleanup()

def nested_finally_break(manager, active, stop):
    with manager:
        while True:
            try:
                while active:
                    if stop:
                        break
                    step()
            finally:
                cleanup()

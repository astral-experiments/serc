def terminal(value):
    with outer():
        try:
            with inner():
                return value
        finally:
            if False:
                note()


def handler(value):
    with outer() as resource:
        try:
            try:
                return value
            except Error:
                pass
        finally:
            if False:
                note()
    after()

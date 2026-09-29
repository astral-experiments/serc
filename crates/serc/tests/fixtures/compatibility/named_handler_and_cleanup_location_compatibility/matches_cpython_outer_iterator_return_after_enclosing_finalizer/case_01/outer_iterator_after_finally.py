def f(items, flag, value):
    for outer in items:
        try:
            try:
                try:
                    work()
                except Error:
                    for item in items:
                        try:
                            return None
                        except InnerError:
                            recover()
            finally:
                cleanup()
        except OuterError:
            recover()
    return value

def f(items, flag, value):
    for outer in items:
        try:
            try:
                try:
                    work()
                except Error as error:
                    for item in items:
                        try:
                            if flag:
                                return value
                        except InnerError:
                            recover()
            finally:
                cleanup()
        except OuterError:
            recover()
    return value

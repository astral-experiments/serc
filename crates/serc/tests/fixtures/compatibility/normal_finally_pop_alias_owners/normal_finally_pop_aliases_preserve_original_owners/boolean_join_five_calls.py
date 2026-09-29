def run(manager, a, b, process):
    with manager:
        try:
            flag = a and b
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass

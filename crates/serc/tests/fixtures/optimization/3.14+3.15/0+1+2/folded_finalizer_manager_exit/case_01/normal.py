def normal():
    with outer():
        try:
            with inner():
                work()
        finally:
            if __debug__:
                note()

def retry():
    while active():
        try:
            work()
            return
        except Error:
            recover()
        finally:
            if resource is not None:
                close(resource)

def f(manager, resource, values, work):
    with manager:
        try:
            work()
        finally:
            if resource is not None:
                try:
                    for value in values:
                        work(value)
                finally:
                    resource.close()

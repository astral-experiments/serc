def close_on_exit(manager, resource):
    with manager:
        try:
            work()
        except Error:
            report()
            return 0
        finally:
            if resource is not None:
                resource.close()

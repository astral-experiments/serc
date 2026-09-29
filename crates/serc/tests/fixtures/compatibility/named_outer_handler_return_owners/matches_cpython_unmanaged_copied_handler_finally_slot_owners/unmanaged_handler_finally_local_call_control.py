def close_on_exit(resource, report):
    try:
        work()
    except Error:
        report()
        return 0
    finally:
        if resource is not None:
            resource.close()

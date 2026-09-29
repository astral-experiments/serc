def retry(managers):
    try:
        for manager in managers:
            with manager:
                return work()
        raise Error
    except Error as error:
        raise Failure from error

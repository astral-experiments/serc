def retry(managers):
    while True:
        try:
            for manager in managers:
                with manager:
                    work()
                    return result()
            raise Error
        except Error as error:
            if failed(error):
                raise
            report(error)

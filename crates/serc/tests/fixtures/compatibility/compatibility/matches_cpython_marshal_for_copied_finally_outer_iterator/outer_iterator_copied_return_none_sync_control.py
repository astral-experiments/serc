def run(items, resource):
    for item in items:
        try:
            value = obtain(item)
            return finish(value).result
        except Error as error:
            if retry(error):
                pause()
        finally:
            if resource is not None:
                close = getattr(resource, "close", None)
                if close is not None:
                    close()

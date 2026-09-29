def run(resource, value):
    result = None
    try:
        result = resource.work(value)
    except Error as error:
        message = str(error)
        report(message)
        assert False
    finally:
        resource.close()
    return convert(result)

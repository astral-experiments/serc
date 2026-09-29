def run(resource, flag):
    def capture():
        return resource
    try:
        if flag:
            prepare(resource)
        consume(resource)
        consume(resource)
        consume(resource)
        return 0
    finally:
        if resource is not None:
            finish(resource)

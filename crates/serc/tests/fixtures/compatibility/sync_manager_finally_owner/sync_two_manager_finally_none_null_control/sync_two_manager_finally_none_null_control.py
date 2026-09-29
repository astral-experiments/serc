def run(first, second, resource, selected):
    try:
        with first, second:
            if selected:
                return resource
            consume(resource)
    except Error:
        raise
    finally:
        if resource is not None:
            resource.close()

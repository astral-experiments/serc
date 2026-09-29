def run(first, second, resource, selected):
    try:
        with first, second:
            if selected:
                return make_result(resource)
            consume(resource)
    except Error:
        raise
    finally:
        if resource is not None:
            resource.close()

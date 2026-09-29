
def name_return_positive(resource, result):
    try:
        return result
    finally:
        if resource is not None:
            resource.close()


def call_positive(resource):
    try:
        return produce()
    finally:
        if resource is not None:
            resource.close()

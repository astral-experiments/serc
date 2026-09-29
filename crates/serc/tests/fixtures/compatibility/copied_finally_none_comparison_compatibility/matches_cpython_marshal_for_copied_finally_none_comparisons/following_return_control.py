
def following_return_control(resource):
    try:
        result = produce()
    finally:
        if resource is not None:
            resource.close()
    return result

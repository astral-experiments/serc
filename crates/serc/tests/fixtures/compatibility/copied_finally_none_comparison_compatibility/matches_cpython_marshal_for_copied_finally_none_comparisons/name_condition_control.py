
def name_condition_control(resource):
    try:
        return produce()
    finally:
        if resource:
            resource.close()

def two_fallthroughs(manager, condition, value, report):
    with manager:
        if condition:
            report(value)
        else:
            report(None)
    return None

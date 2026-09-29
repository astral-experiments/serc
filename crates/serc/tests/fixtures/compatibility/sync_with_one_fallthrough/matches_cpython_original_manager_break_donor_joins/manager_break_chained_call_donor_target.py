def send(values, manager):
    for value in values:
        with manager(value):
            if value.ready:
                return value.result
            elif value.retry:
                retry(value)
            else:
                report(value)
                break
    return None

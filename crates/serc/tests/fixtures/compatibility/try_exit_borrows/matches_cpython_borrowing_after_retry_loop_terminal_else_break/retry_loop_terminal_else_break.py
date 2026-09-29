def retry(check, work, recover, finish, value):
    while True:
        try:
            value = work(value)
            if check(value):
                recover(value)
            else:
                break
        except ValueError:
            recover(value)
    finish(value)
    return value

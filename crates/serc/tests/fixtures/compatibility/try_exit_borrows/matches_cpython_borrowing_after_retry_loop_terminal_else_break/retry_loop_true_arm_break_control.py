def retry(check, work, recover, finish, value):
    while True:
        try:
            value = work(value)
            if check(value):
                break
            else:
                recover(value)
        except ValueError:
            recover(value)
    finish(value)
    return value

def retry(check, work, update, finish, value):
    while True:
        try:
            value = work(value)
            if check(value):
                value = update(value)
            else:
                break
        except ValueError:
            value = yield
    finish(value)
    return value

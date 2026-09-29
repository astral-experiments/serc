def example(active, choose, prepare, recover, finish, guard):
    result = 1
    try:
        prepare()
        while True:
            result = prepare()
            if choose(result):
                break
        else:
            recover("done" if guard else "empty")
            result = 0
    except ValueError as error:
        recover(error)
        result = 2
    finally:
        if guard is not None:
            finish(result)
    return result

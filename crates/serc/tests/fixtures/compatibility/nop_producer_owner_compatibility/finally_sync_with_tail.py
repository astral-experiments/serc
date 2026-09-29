try:
    prepare()
finally:
    with manager:
        work()
        pass

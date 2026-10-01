def local_condition(first, second, target):
    with manager(), manager(), manager(), manager():
        try:
            if first and second:
                consume(target)
            else:
                consume("first" if first else "second")
        except Failure as error:
            report(error)
        else:
            report(target)


def captured_condition(first, second, target):
    def capture():
        return first

    with manager(), manager(), manager(), manager():
        try:
            if first and second:
                consume(target)
            else:
                consume("first" if first else "second")
        except Failure as error:
            report(error)
        else:
            report(target)

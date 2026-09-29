def capture(subject):
    global captured
    match subject:
        case captured:
            pass
    consume(subject)


def capture_as(subject):
    global captured_as
    match subject:
        case _ as captured_as:
            pass
    consume(subject)

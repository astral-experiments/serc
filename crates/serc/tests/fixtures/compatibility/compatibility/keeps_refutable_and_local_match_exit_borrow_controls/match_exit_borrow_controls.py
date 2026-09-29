def refutable_global_capture(subject):
    global captured
    match (None, subject):
        case (_, captured):
            pass
    consume(subject)


def nested_local_capture(subject):
    match subject:
        case outer:
            match subject:
                case inner:
                    pass
    consume(subject, outer, inner)

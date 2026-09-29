def f(kind, value):
    match kind:
        case 'message':
            match value:
                case (False, error):
                    yield error
                    return
                case (True, result, recipient):
                    consume(recipient)
                    yield result, recipient

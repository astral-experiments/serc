def f(value):
    match value:
        case Event(at=_, message=message, exception=_, request_id=_, raw=_):
            return message
    return None

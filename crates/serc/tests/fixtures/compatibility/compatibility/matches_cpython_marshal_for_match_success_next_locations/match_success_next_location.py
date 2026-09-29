def tuple_result(subject):
    match subject:
        case 1:
            return make_first(), make_second()
        case 2:
            raise Failure()
        case _:
            return None

async def await_result(subject):
    match subject:
        case 1:
            value = await produce()
            return value
        case 2:
            await consume()
        case _:
            return None

def nested_match(subject, nested):
    match subject:
        case 1:
            match nested:
                case 2:
                    return [nested]
                case _:
                    return []
        case _:
            return None

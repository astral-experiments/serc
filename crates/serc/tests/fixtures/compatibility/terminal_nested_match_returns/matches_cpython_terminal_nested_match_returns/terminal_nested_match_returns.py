def ordinary_case(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    consume("first")
                case 2:
                    consume("second")
        case _:
            pass


def wildcard_outer(kind, value):
    match kind:
        case "skip":
            return "skip"
        case _:
            prepare(value)
            match value:
                case 1:
                    consume("first")
                case 2:
                    consume("second")


def wildcard_inner(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    consume("first")
                case _:
                    pass


def capture_raise(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    consume("first")
                case 2:
                    consume("second")
                case other:
                    raise ValueError(other)


def class_as_cases(kind, value):
    match kind:
        case Envelope():
            match value:
                case First() as first:
                    consume(first)
                case Second() as second:
                    consume(second)
                case _:
                    raise ValueError(value)


def unmatched_or_failure(kind, value):
    match kind:
        case "message":
            match value:
                case First():
                    return "first"
                case (
                    Second()
                    | Third()
                ):
                    return "other"


def generator_or_cases(kind, value):
    match kind:
        case "skip":
            return
        case _:
            match value:
                case "first" | "second":
                    result = produce(value)
                    yield result
                case "last":
                    result = produce(value)
                    yield result


def generator_sequence_cases(kind, value):
    match kind:
        case "message":
            match value:
                case (False, error):
                    yield error
                    return
                case (True, result, recipient):
                    consume(recipient)
                    yield (
                        result,
                        recipient,
                    )


async def coroutine_cases(kind, value):
    match kind:
        case "message":
            prepared = prepare(value)
            match prepared:
                case "first":
                    await consume("first")
                case "second":
                    await consume("second")
                case "third":
                    await consume("third")
                case other:
                    raise ValueError(other)


def nested_three_levels(kind, value, detail):
    match kind:
        case "message":
            match value:
                case "nested":
                    match detail:
                        case 1:
                            consume("first")
                        case 2:
                            consume("second")
                case _:
                    consume("outer default")


def guarded_inner(kind, value, flag):
    match kind:
        case "message":
            match value:
                case 1 if flag:
                    consume("first")
                case _ if check(value):
                    consume("guarded default")


def constant_false_inner(kind, value):
    match kind:
        case "message":
            match value:
                case 1 if False:
                    consume("unreachable")
                case 2:
                    consume("second")


def constant_true_inner(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    consume("first")
                case captured if True:
                    consume(captured)


def false_guarded_default(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    consume("first")
                case _ if False:
                    pass


def pass_and_assignment(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    pass
                case 2:
                    result = "second"
                case _:
                    pass


def explicit_return_inner(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    return "first"
                case _:
                    return "default"


def explicit_raise_inner(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    raise ValueError("first")
                case _:
                    raise ValueError("default")


def following_case_statement_control(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    consume("first")
                case 2:
                    consume("second")
            after(value)


def following_match_statement_control(kind, value):
    match kind:
        case "message":
            match value:
                case 1:
                    consume("first")
                case 2:
                    consume("second")
    after(kind)


def single_match_control(value):
    match value:
        case 1:
            consume("first")
        case 2:
            consume("second")


def trailing_if_control(kind, value):
    match kind:
        case "message":
            if value:
                consume("first")
            else:
                consume("second")


def folded_true_following_control(kind, value):
    match kind:
        case "message":
            if True:
                match value:
                    case 1:
                        consume("first")
                    case 2:
                        consume("second")
            after(value)


def folded_false_following_control(kind, value):
    match kind:
        case "message":
            if False:
                consume("unreachable")
            else:
                match value:
                    case 1:
                        consume("first")
                    case 2:
                        consume("second")
            after(value)


def protected_try(kind, value):
    try:
        match kind:
            case "message":
                match value:
                    case 1:
                        consume("first")
                    case 2:
                        consume("second")
    except ValueError:
        recover(value)


def protected_with(kind, value, manager):
    with manager:
        match kind:
            case "message":
                match value:
                    case 1:
                        consume("first")
                    case 2:
                        consume("second")


async def protected_async_with(kind, value, manager):
    async with manager:
        match kind:
            case "message":
                match value:
                    case 1:
                        await consume("first")
                    case 2:
                        await consume("second")


def deferred_constant_order(kind, value):
    match kind:
        case "message":
            before = "before nested"
            match value:
                case 1:
                    result = "first"
                case 2:
                    result = "second"
        case "later":
            result = "after nested"

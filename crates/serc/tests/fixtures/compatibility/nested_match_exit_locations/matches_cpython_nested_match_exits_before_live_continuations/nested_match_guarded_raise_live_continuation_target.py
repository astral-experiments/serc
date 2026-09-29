def collect(items, expected):
    result = []
    for item in items:
        match item.kind:
            case "reply":
                match item.channel:
                    case "thought":
                        if item.recipient == expected:
                            pass
                        elif item.recipient.startswith("tool"):
                            pass
                        else:
                            raise ValueError("recipient")
                    case "answer":
                        if item.recipient != expected:
                            raise ValueError("answer recipient")
                    case _:
                        raise ValueError("channel")
            case "tool":
                if item.recipient != expected:
                    raise ValueError("tool recipient")
            case _:
                pass
        result.append(item)
    return result

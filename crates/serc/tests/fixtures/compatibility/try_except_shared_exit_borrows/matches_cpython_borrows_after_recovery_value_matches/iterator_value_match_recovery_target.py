def collect(messages, decode, report):
    seen = []
    for message in messages:
        first = False
        second = False
        try:
            values = decode(message)
            for key, items in values.items():
                size = len(items) if isinstance(items, list) else 1
                report(key, size)
                if key.startswith("alpha"):
                    first = True
                elif key.startswith("beta"):
                    second = True
                elif key == "other":
                    first = True
        except Exception:
            for line in message.splitlines():
                key, *values = line.split("|")
                if values:
                    report(key, len(values))
                match key:
                    case "slow":
                        first = True
                    case "fast":
                        second = True
        report(message, first, second)
        seen.append((first, second))
    return seen

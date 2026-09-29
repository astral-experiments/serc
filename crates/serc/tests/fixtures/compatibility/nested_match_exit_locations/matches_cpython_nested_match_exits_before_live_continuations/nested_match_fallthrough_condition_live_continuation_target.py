def parse(kind, mode, raw, parse_one, extract, report):
    result = None
    match kind:
        case "image":
            if mode == "raw":
                result = parse_one(raw)
                other = extract(raw)
                if other is None:
                    report("none")
                else:
                    result_two = parse_one(other)
                    if result_two != result:
                        report("mismatch")
            else:
                extracted = extract(raw)
                if extracted is None:
                    report("empty")
                else:
                    result = parse_one(extracted)
        case "video":
            result = parse_one(raw)
        case _:
            report("unsupported")
    if result is not None:
        report(result)
    return result

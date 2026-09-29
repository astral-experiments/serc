def parse(items, groups, kind):
    match kind:
        case "other":
            yield 0
            return
        case "wanted":
            if groups:
                for item in groups:
                    if item:
                        break
    for item in items:
        yield item

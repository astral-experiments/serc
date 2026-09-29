def parse(items, groups, kind, visit):
    match kind:
        case "other":
            yield 0
            return
        case "wanted":
            if groups:
                visit(groups)
    for item in items:
        yield item

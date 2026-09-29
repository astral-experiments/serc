def run(items, flags, record):
    for item in items:
        if flags["outer"]:
            if flags["middle"]:
                if flags["inner"]:
                    record["first"] += item
                else:
                    record["second"] += item
                    if flags["left"] and flags["right"]:
                        record["shared"] += item
            else:
                record["fallback"] += item
        else:
            record["other"] += item
        if flags["after"]:
            record["after"] += item
    return record

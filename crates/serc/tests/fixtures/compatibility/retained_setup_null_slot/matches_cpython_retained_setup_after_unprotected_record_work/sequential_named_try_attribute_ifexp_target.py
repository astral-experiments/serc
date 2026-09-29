def run(groups, allowed):
    output = {}
    records = {}
    for group_index, group in enumerate(groups):
        for item in group:
            if item.kind in allowed:
                if group_index not in output:
                    output[group_index] = []
                try:
                    first = transform(item.value) if item.value else None
                except AttributeError as error:
                    report(item.value)
                    report(error)
                    first = None
                output[group_index].append(Entry(value=item.value, filtered=first))
                if item.key not in records:
                    records[item.key] = Record(
                        before=None, after=None, weight=None,
                        key=item.key, value=None, latest=None,
                    )
                    records[item.key].before = Content(
                        value=load_before(item.value), key=item.key,
                    )
                records[item.key].latest = parse(item.value)
                records[item.key].after = Content(
                    value=load_after(item.value), key=item.key,
                )
                records[item.key].weight = measure(item.value)
                merged = combine(
                    records[item.key].before.value,
                    records[item.key].after.value,
                    item.key,
                )
                try:
                    second = transform(item.value) if item.value else None
                except AttributeError as error:
                    report(item.value)
                    report(error)
                    second = None
                records[item.key].value = parse(merged)
                records[item.key].filtered = second
    return records, output

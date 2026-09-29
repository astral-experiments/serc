def collect(rows, shallow, limit):
    entries = []
    groups = []
    stopped = False
    try:
        if shallow:
            for row in rows:
                name = row.name
                if not name:
                    continue
                if name.endswith("/"):
                    groups.append({"kind": "group", "name": name})
                else:
                    entry = convert(row)
                    if entry is not None:
                        entries.append(entry)
                if len(entries) + len(groups) >= limit:
                    stopped = True
                    break
        else:
            for row in rows:
                entry = convert(row)
                if entry is None:
                    continue
                entries.append(entry)
                if len(entries) >= limit:
                    stopped = True
                    break
    except Exception:
        recover()
    return entries, groups, stopped

def collect(records):
    try:
        output = []
        for record in records:
            if record:
                try:
                    value = decode(record)
                    output.append({"label": value.label if value.label else ""})
                except Exception:
                    continue
        return output
    except Exception:
        raise

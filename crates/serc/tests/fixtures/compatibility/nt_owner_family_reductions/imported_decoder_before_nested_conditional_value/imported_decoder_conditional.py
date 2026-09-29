from adapters import Decoder

def collect(records):
    try:
        output = []
        for record in records:
            if record:
                try:
                    value = Decoder.decode(record)
                    output.append({
                        "key": record,
                        "prefix": value.label.split()[0],
                        "label": value.label if value.label else "",
                    })
                except Exception:
                    continue
        return output
    except Exception:
        raise

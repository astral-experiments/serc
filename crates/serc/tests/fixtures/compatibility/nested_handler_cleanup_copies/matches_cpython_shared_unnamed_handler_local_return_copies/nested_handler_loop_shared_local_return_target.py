def collect_entries(data):
    entries = []
    try:
        for key in data['sources'].keys():
            item = data['sources'][key]
            entries.append({'value': item['value'], 'kind': item['kind']})
    except Exception:
        try:
            for item in data['sources']:
                entries.append({'value': item['value'], 'kind': item['kind']})
        except Exception:
            try:
                for key in data.keys():
                    item = data[key]
                    entries.append({'value': item['value'], 'kind': item['kind']})
            except Exception:
                pass
    return entries

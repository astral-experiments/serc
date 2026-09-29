def collect_entries(data):
    entries = []
    try:
        for key in data['sources'].keys():
            item = data['sources'][key]
            entries.append({'value': item['value'], 'kind': item['kind']})
    except Exception:
        pass
    return entries

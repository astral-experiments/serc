def collect(record):
    result = []
    match record['kind']:
        case 'first' | 'second' | 'third':
            if entries := record.get('entries'):
                result = [entry['value'] for entry in entries.values() if entry and entry['value']]
        case 'items':
            for item in record.get('items', []):
                if (value := item.get('value')) is not None:
                    result.append(value)
    return list(map(normalize, result))

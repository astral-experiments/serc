def probe(fetch):
    while True:
        result = fetch()
        if isinstance(result, dict):
            for item in result.get('items', []):
                yield item
            marker = result.get('next')
            if not marker:
                break
        else:
            for item in result:
                yield item
            break

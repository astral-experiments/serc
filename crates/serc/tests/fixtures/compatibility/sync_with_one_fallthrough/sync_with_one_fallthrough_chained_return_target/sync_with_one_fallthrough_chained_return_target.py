def fetch(retries, client_factory, method, endpoint, stats):
    tags = []
    for attempt in retries:
        with attempt:
            with client_factory() as client:
                request = client.build_request(method, endpoint)
                response = client.send(request)
                tags.extend([f'status:{response.status_code}', 'pool:false'])
                stats.timing(endpoint, tags=tags)
                if response.status_code == 200:
                    return response.json()
                elif method.upper() == 'GET' and response.status_code == 404:
                    return None
                else:
                    response.raise_for_status()
    return None

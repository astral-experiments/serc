def poll(url, timeout, interval):
    start = clock.time()
    last_error = None
    while clock.time() - start < timeout:
        try:
            response = client.get(url, timeout=0.5)
            if 200 <= response.status < 500:
                return
        except Exception as error:
            last_error = error
        clock.sleep(interval)
    raise TimeoutError(url) from last_error

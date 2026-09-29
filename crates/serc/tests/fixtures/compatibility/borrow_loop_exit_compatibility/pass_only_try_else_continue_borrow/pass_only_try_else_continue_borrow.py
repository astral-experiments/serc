def run(client, delays, stop):
    for delay in delays:
        try:
            client.send()
        except Error:
            pass
        else:
            if stop:
                continue
        consume(delay)
        consume(client)

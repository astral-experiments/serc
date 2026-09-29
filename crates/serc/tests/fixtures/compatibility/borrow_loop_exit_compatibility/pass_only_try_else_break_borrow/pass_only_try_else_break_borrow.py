def run(client, delays, stop):
    for delay in delays:
        try:
            client.send()
        except Error:
            pass
        else:
            if stop:
                break
        consume(delay)
        consume(client)

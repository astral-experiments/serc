def run(ready, poll, read, send, stop, report, tick):
    try:
        while ready():
            if poll():
                data = read()
                if data:
                    response = send(data)
                    if response.status_code != 200:
                        report(response)
                        stop()
                        break
                else:
                    break
            tick()
    except Exception as error:
        report(error)

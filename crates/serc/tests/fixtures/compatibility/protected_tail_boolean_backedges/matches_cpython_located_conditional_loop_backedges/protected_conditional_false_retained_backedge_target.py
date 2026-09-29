def run(ready, poll, read, send, stop, report):
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
    except Exception as error:
        report(error)

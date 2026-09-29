def run(values, accepted, exists, remove, client):
    try:
        for value in values:
            try:
                if accepted(value):
                    return
            finally:
                if value and exists(value):
                    try:
                        remove(value)
                    except OSError:
                        pass
    finally:
        try:
            client.disconnect()
        except OSError:
            pass

def run(client):
    while True:
        try:
            raise Error
        except Error as error:
            if should_stop(error):
                break
            raise
        finally:
            if client is not None:
                client.close()

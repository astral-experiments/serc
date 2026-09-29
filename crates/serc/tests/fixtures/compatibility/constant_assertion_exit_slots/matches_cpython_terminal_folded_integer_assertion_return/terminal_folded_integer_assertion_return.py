def check_status(client, blocked):
    if blocked:
        client.block()
    response = client.get()
    assert response.status_code == 503 if blocked else 200

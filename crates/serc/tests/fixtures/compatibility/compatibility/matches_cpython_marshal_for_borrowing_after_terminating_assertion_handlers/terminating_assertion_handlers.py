def following_assertion(client):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert False, f"unexpected error: {error}"
    assert response.status_code == 200


def multiple_assertions(client):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert False, f"unexpected error: {error}"
    assert response.status_code == 200
    assert response.json() == {"name": "value"}


def following_call(client, consume):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert False, f"unexpected error: {error}"
    consume(response)


def following_assignment(client):
    try:
        response = client.get("/thing")
    except Exception:
        assert 0, "unexpected error"
    status = response.status_code
    consume(status)


def dynamic_assertion_control(client, condition):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert condition, f"unexpected error: {error}"
    assert response.status_code == 200


def true_assertion_control(client):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert True, f"unexpected error: {error}"
    assert response.status_code == 200


def multiple_handler_control(client):
    try:
        response = client.get("/thing")
    except Exception as error:
        assert False, f"unexpected error: {error}"
    except BaseException as error:
        consume(error)
    assert response.status_code == 200


def preceding_handler_statement_control(client, consume):
    try:
        response = client.get("/thing")
    except Exception as error:
        consume(error)
        assert False, f"unexpected error: {error}"
    return response.status_code


def exception_group_control(client):
    try:
        response = client.get("/thing")
    except* Exception as error:
        assert False, f"unexpected error: {error}"
    assert response.status_code == 200


def nested_name_return_control(client):
    try:
        try:
            response = client.get("/thing")
        except Exception as error:
            assert False, f"unexpected error: {error}"
        return response
    except LookupError:
        raise


def nested_attribute_return_control(client):
    try:
        try:
            response = client.get("/thing")
        except Exception as error:
            assert False, f"unexpected error: {error}"
        return response.status_code
    except LookupError:
        raise

def sample(request, consume):
    try:
        request()
    except Error as error:
        if "available":
            consume(error)
    return None

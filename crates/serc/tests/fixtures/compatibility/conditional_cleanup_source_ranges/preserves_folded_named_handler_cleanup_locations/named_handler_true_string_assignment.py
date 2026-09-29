def sample(request, ready):
    response = None
    try:
        response = request()
    except Error as error:
        if "available":
            response = None
    return response

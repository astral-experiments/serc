def sample(request, ready):
    response = None
    try:
        response = request()
    except Error as error:
        if "":
            response = None
    return response

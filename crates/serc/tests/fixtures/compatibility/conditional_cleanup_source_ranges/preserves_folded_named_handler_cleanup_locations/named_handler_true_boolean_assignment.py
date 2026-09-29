def sample(request, ready):
    response = None
    try:
        response = request()
    except Error as error:
        if True:
            response = None
    return response

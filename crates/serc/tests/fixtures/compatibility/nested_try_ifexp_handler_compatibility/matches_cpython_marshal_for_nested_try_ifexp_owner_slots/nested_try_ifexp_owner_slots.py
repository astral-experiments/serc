def name_final_clean(check, work, response):
    try:
        try:
            value = work()
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
    except Exception as error:
        result = error
    return result


def name_final_slot(check, work, response):
    try:
        try:
            value = work()
            pair = (value, response)
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
    except Exception as error:
        result = error
    return result


def attribute_final_clean(check, work, response):
    try:
        try:
            value = work()
            response.result = "OK" if check(value) else value
        except Exception as error:
            response.result = error
    except Exception as error:
        response.result = error
    return response


def attribute_final_slot(check, work, response):
    try:
        try:
            value = work()
            pair = (value, response)
            response.result = "OK" if check(value) else value
        except Exception as error:
            response.result = error
    except Exception as error:
        response.result = error
    return response


def name_nonfinal_clean(check, work, response):
    try:
        try:
            value = work()
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
        response.result = result
    except Exception as error:
        result = error
    return response


def name_nonfinal_slot(check, work, response):
    try:
        try:
            value = work()
            pair = (value, response)
            result = "OK" if check(value) else value
        except Exception as error:
            result = error
        response.result = result
    except Exception as error:
        result = error
    return response


def name_nonfinal_after_definition():
    from package.module import is_pyobject_tainted

    response = ResultResponse(request.args.get("package_param"))

    try:
        parameter = request.args.get("package_param", "default-value")

        @log_function_call
        def sample_function(param):
            return f"Function executed with param: {param}"

        try:
            result = sample_function(parameter)
            result_output = (
                "OK" if is_pyobject_tainted(result) else f"Error: result is not tainted: {result}"
            )

        except Exception as error:
            result_output = f"Error: {str(error)}"

        response.result = result_output
    except Exception as error:
        response.result = f"Error: {str(error)}"

    return response.done()

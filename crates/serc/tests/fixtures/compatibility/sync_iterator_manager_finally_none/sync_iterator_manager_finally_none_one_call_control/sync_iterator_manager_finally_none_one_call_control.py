def run(manager, response, chunks, selected):
    try:
        with manager:
            for chunk in chunks:
                if selected:
                    return make_result(response)
    except Error:
        raise
    finally:
        if response is not None:
            response.close()

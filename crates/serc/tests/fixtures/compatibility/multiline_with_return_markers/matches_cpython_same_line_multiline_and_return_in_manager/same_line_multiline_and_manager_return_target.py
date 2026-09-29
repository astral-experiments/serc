def respond(manager, retained, response, completed):
    with manager:
        if completed:
            return retained is not None and (
                retained.status == response.status
                and retained.read() == response.body
            )
        return True

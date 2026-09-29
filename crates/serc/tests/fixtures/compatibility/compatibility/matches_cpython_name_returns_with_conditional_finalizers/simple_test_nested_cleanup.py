def mutate(client, resource, credential, attempts):
    try:
        for attempt in range(attempts):
            current = read(resource)
            updated = normalize(current)
            try:
                write(resource, updated)
            except (Conflict, AlreadyExists) as error:
                last_error = error
                continue
            return updated
        raise Failure from last_error
    finally:
        if resource is not None:
            resource.close()
            if credential is not None:
                credential.close()

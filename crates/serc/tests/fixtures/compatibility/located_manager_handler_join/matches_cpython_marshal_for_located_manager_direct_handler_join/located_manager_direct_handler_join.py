def ensure_support(connection, errors, report):
    with connection.open() as resource:
        try:
            resource.execute('probe')
            resource.fetchone()
            return
        except errors.Failure as error:
            report(f'Feature unavailable: {error}')

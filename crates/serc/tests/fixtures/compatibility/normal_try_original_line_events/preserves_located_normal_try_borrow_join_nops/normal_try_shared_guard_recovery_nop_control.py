def checked_pair(first, second, recover, finish):
    try:
        if first() and second():
            raise ValueError('rejected')
    except Exception as error:
        try:
            recover(error)
        except OSError:
            return None
        return False
    finish()

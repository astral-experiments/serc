def protected_step(check, recover, finish):
    try:
        try:
            if not check():
                raise ValueError('rejected')
        except Exception as error:
            try:
                recover()
            except OSError as recovery_error:
                raise RuntimeError('recovery failed') from error
            raise
        finish()
    except Exception as error:
        raise RuntimeError('failed') from error

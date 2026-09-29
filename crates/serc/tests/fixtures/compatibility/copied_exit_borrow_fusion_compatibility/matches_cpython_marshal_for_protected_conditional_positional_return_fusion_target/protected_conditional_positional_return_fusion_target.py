import service

def f(text, *, direct, sensitive):
    if not text:
        raise ValueError('text is required')
    options = 0 if sensitive else service.DEFAULT
    try:
        return service.compile(text if direct else service.escape(text), options)
    except service.Error as error:
        raise ValueError(f'invalid pattern: {error}') from error

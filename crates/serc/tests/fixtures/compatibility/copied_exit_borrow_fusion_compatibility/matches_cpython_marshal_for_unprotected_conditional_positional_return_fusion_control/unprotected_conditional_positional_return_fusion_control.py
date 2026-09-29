import service

def f(text, *, direct, sensitive):
    if not text:
        raise ValueError('text is required')
    options = 0 if sensitive else service.DEFAULT
    return service.compile(text if direct else service.escape(text), options)

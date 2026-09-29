def tuple_value(label, url):
    return label, lambda: url

def keyword_call(label, url):
    return register(label=label, command=lambda: open(url))

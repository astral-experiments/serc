module_name = "<module>"

def outer():
    def nested():
        pass
    return "outer.<locals>.nested"

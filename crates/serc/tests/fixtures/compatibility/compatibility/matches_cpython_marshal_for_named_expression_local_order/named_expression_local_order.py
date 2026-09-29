def chained():
    return (outer := (inner := value))

lambda_body = lambda: (lambda_outer := (lambda_inner := lambda_value))

def mixed_global():
    global global_outer
    result = (global_outer := (local_inner := local_value))
    local_value = 1
    return result

def list_element_late_load(items):
    result = [(outer := (inner := seed)) for item in items]
    seed = 1
    return result

def list_two_filters(items):
    result = [item for item in items if (first := left) if (second := right)]
    left = right = 1
    return result

def set_element(items):
    result = {(outer := (inner := seed)) for item in items}
    seed = 1
    return result

def dict_key_then_value(items):
    result = {
        (key_target := key_seed): (value_target := value_seed)
        for item in items
    }
    key_seed = value_seed = 1
    return result

def nested_list(rows):
    result = [[(outer := (inner := seed)) for item in row] for row in rows]
    seed = 1
    return result

def generator_reverse_names(items):
    return ((a_outer := (z_inner := item)) for item in items)

def decorated():
    @(decorator_outer := (decorator_inner := decorator_value))
    def nested():
        pass

    decorator_value = 1
    return nested

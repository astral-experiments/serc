global module_values, module_callable
module_explicit_global = consume(*module_values)
module_load_name = consume(*module_name)
module_both_global = module_callable(*module_values)
module_multiline = consume(
    *module_values
)
module_named_keyword = consume(*module_values, option=True)

module_direct_global = module_callable()

class ClassScope:
    global class_values, class_callable
    explicit_global = consume(*class_values)
    load_name = consume(*class_name)
    both_global = class_callable(*class_values)
    named_keyword = consume(*class_values, option=True)

    direct_global = class_callable()


class AnnotationScope:
    type Alias = consume(*annotation_values)


def global_callable():
    return consume(*values)

def local_callable(consume):
    return consume(*values)

def local_argument(values):
    return consume(*values)

def bound_callable(obj):
    return obj.consume(*values)

def computed_callable():
    return make_consumer()(*values)

def comprehension_shadow():
    global values
    return [consume(*values) for values in collections]

def called_argument():
    return consume(*make_values())

def leading_argument():
    return consume(1, *values)

def multiple_arguments():
    return consume(*values, *more_values)

def keyword_unpacking():
    return consume(*values, **options)

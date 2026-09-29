def ordering():
    @decorator
    def function(positional=(function_target := function_source), *, keyword=keyword_source):
        pass

    @class_decorator
    class Class((base_target := base_source), metaclass=metaclass_source):
        pass

    function_source = keyword_source = None
    base_source = object
    metaclass_source = type
    decorator = class_decorator = identity


def function_default_closure(value):
    def parent():
        def plain(item=value):
            return item

        def generic[T](item=value):
            return item

        return plain, generic

    return parent


def class_decorator_closure(decorator):
    def parent():
        @decorator
        class Child:
            pass

        return Child

    return parent

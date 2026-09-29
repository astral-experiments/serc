def ternary(name, suffix):
    return name[:-len(suffix)] if name.endswith(suffix) else name

def simple(condition, left, right):
    return left if condition else right

def terminal_attribute(condition, holder, left, right):
    holder.value = left.value if condition else right

def boolean(left, right):
    return left or right

def nested(condition, name):
    return name.upper() if condition else name

def nested_boolean(left, middle, right):
    return (left and middle) or right, (left or middle) and right

def folded_annotation(arg: 2 or None | int):
    pass

lambda_conditional = lambda condition, left, right: left if condition else right
lambda_boolean = lambda left, right: left or right
module_boolean = (left and middle) or right

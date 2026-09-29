[value for value in values]
{key: (left, right) for key, (left, right) in values}
module_result = {last := value for value in range(2)}
nested_filter = [(key, value) for key, value in pairs if key in [field.name for field in fields]]
nested_iterable = [(left, right) for left in values for right in [item.value for item in items]]
generator_assignment = list((captured := value for value in values))

def f():
    exponential, base_multiplier = 1, 2
    hash_map = {
        (exponential := (exponential * base_multiplier) % 3): i + 1
        for i in range(2)
    }
    return hash_map

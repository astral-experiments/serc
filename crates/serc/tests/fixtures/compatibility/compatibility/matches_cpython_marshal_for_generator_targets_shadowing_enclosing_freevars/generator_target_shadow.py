def lambda_parent(all_elements, element):
    return lambda owner: all(owner.contains(element) for element in all_elements)

def function_parent(all_elements, element):
    def child(owner):
        return all(owner.contains(element) for element in all_elements)
    return child

def lambda_in_generator_parent(items, predicate):
    def child():
        return any((lambda: predicate(item))() for item in items)
    return child

def comprehension_shadow_parent(items, captured):
    def child():
        values = [captured for captured in items]
        generated = (item for item in items)
        return captured, values, generated
    return child

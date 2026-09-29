def attribute_chain(x, obj):
    if 0 < x < 10:
        obj.value = x

def name_chain(x):
    if 0 < x < 10:
        result = x

def subscript_chain(x, items):
    if 0 < x < 10:
        items[0] = x

def augmented_attribute_chain(x, obj):
    if 0 < x < 10:
        obj.value += x

def augmented_name_chain(x, obj):
    if 0 < x < 10:
        x += 1

def augmented_subscript_chain(x, obj):
    if 0 < x < 10:
        obj[0] += x

def augmented_slice_chain(x, obj):
    if 0 < x < 10:
        obj[0:2] += obj

async def async_attribute_chain(x, obj):
    if 0 < x < 10:
        obj.value = x

def attribute_single(x, obj):
    if x < 10:
        obj.value = x

def attribute_chain_nonterminal(x, obj):
    if 0 < x < 10:
        obj.value = x
    result = x

def explicit_return_chain(x, obj):
    if 0 < x < 10:
        return x

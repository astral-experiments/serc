value: int
[x for x in values if True]
[x for x in values if not None]
[result for x in values if not predicate]
{target.attr: None for target.attr in values}
[
    result
    for x in values
    if True
]
[
    result
    for x in values
    if x
    if not None
]
[x for x in values if left and right]
{x for x in values if lower < x < upper if accepted}
(x for x in values if left and right if accepted)
(x for x in values if left and right async for y in async_values if accepted)

def restored_then_subscript(values):
    result = {}
    result['key'] = [value for value in values]

def assignment_reuses_target():
    tasks = [task for task in tasks]

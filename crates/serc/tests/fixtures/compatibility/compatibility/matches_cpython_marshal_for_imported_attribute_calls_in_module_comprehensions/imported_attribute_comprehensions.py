import re
import re as captured_module
import re as inner_only_module
import re as lambda_default_module
import re as lambda_module
import re as later_target_module
import re as outer_lambda_module
import re as outer_target_module
import re as target_module
import re as target_expression_module
import re as walrus_module

ordinary = re.match("a", "a")
first_iterable = [item for item in re.split(",", "a,b")]
filtered = [item for item in ("a", "b") if re.match("a", item)]
elements = [re.split(",", item) for item in ("a,b", "c,d")]
dictionary = {item: re.compile(item) for item in ("a", "b")}
unique = {re.escape(item) for item in ("a", "b")}
nested = [
    re.match("a", inner)
    for outer in (1,)
    for inner in re.split(",", "a,b")
]
generator = (re.match("a", item) for item in ("a", "b"))

target_shadow = [
    target_module.match("a", "a")
    for target_module in (target_module,)
]
later_target_shadow = [
    later_target_module.match("a", "a")
    for item in (1,)
    for later_target_module in (later_target_module,)
]
target_expression = [
    None
    for target_expression_module.match("a", "a").target in (1,)
]
captured_target = [
    (lambda: captured_module.match("a", "a"))
    for captured_module in (captured_module,)
]
walrus_shadow = [
    walrus_module.match("a", item)
    for item in ("a",)
    if (walrus_module := walrus_module)
]
inner_only_values = (inner_only_module,)
inner_target_only = [
    [
        inner_only_module.match("a", "a")
        for inner_only_module in inner_only_values
    ]
    for outer in (1,)
]
outer_target_inner_free = [
    [outer_target_module.match("a", "a") for inner in (1,)]
    for outer_target_module in (outer_target_module,)
]
lambda_only = [
    (lambda: lambda_module.match("a", "a"))
    for item in (1,)
]
lambda_default = [
    (lambda value=lambda_default_module.match("a", "a"): value)
    for item in (1,)
]
outer_free_and_lambda = [
    (
        outer_lambda_module.match("a", "a"),
        lambda: outer_lambda_module.match("a", "a"),
    )
    for item in (1,)
]

def function_scope():
    return [re.match("a", item) for item in ("a", "b")]

lambda_else = lambda outer, inner, first, second, third: (
    first if outer else second if inner else third
)
lambda_body = lambda outer, inner, first, second, third: (
    (first if inner else second) if outer else third
)


def assignment_control(holder, outer, inner, first, second, third):
    holder.value = (
        (first if inner else second) if outer else third
    )

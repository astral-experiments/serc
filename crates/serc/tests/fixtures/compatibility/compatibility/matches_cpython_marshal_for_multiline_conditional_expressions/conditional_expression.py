value = (
    'body'
    if True
    else 'otherwise'
)

def nested():
    target.attribute = (
        (first if inner else second)
        if outer
        else third
    )

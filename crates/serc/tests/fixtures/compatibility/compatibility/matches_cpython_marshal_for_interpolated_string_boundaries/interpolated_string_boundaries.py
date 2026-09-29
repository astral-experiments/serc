
def render(item, nested):
    rf'raw\path {item}'
    f'''first
{item!r}
# literal
last'''
    r'left' f"middle {item}" '''right'''
    t'left' tr"""right {item}
# literal"""
    f"""start {(
        {'}': nested['{']}  # expression comment
    ) = } end"""
    t"""{(
        nested["key"]  # expression comment
    ) = }"""

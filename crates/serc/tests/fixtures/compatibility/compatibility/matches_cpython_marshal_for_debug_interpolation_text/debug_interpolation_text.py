
def render(item, mapping, width):
    f"{  item   =   !r}"
    t"{  item   =   !r}"
    t"""{(
        mapping["="]  # = inside comment
    )   =   :>{width}}"""
    t"""{f"{item=} = literal" = !s:^10}"""
    f"""{item=  # after marker
}"""
    t"""{item=  # after marker
}"""

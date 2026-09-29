
def render(value):
    f"""{(
        '# single',
        "# double",
        r'# raw',
        '''# triple''',
        """# triple double""",
    ) =  # debug comment
    }"""
    t"""{"escaped \\\" # still string" = }"""
    t"""{f"nested # {value}" = !r:^20}"""
    t"""{(
        '# literal',
        f"nested # {value}",
    )  # expression comment
    !r:^20}"""

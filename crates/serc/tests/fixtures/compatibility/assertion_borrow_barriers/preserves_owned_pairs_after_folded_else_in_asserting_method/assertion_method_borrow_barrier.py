class Projection(Base):
    def __init__(self, width, heads, groups, head_width=128):
        super().__init__()
        assert heads % groups == 0
        output_width = heads * head_width
        grouped_width = groups * head_width
        if False:
            self.combined = make(width, output_width + grouped_width)
        else:
            self.query = make(width, output_width)
            self.grouped = make(width, grouped_width + grouped_width)
        self.rotary = rotate(head_width)
        self.output = make(output_width, width)

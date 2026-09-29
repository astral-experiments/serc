def configure():
    global needle
    needle = "p"
before = needle
result = [value for value in values_for(needle) if needle in value and value.startswith("p_")]
after = needle

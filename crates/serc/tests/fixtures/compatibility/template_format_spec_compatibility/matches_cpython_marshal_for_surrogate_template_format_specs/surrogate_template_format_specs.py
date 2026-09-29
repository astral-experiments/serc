def regular(value):
    return t"{value:\ud800}"

def dynamic(value, width):
    return t"{value:\ufffd\ud800{width}\udfff�}"

def raw(value):
    return tr"{value:\ud800�}"

def raw_dynamic(value, width):
    return rt"{value:\ud800�{width}\udfff}"

def raw_then_regular(value):
    return tr"{value:\ud800�}" t"{value:\U0000dfff}"

def regular_then_raw(value, width):
    return t"{value:\ud800{width}}" tr"{value:\udfff�{width}}"

def nested_regular(value, echo):
    return t"{value:{echo:\ud800�}}"

def nested_raw(value, echo):
    return tr"{value:{echo:\ud800�}}"
